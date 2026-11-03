//! The window's sound, for window sharing. Two sources:
//!
//! - **The app's sound** (the default): Windows' process loopback (WASAPI,
//!   `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`) captures what this
//!   process and its children play. WebView2's processes are children of
//!   the app, so that is exactly the window's sound: YouTube, My music...
//!   and nothing from other programs (no Discord, no game, no
//!   notifications). Needs Windows 10 build 20348 or later (Windows 11).
//! - **All of the PC's sound** (only when the host asks for it, for one
//!   sharing session): the default output's loopback, which works on any
//!   Windows 10 but also carries calls, notifications and other programs.
//!   The panel offers it when the app's sound can't be captured or is not
//!   heard while the page plays (see [`SoundWatch`]), and says what it
//!   includes.
//!
//! The sound is 48 kHz stereo, 16-bit; it goes out in 20 ms pieces. If the
//! output refuses that format, its own (usually 32-bit float) is converted
//! here ([`Converter`]).
//!
//! Threads: `ActivateAudioInterfaceAsync` is called on the app's main (UI)
//! thread, as Microsoft requires (from another thread Windows may never
//! answer). Its answer arrives on a Windows worker thread and is handed to
//! the capture thread. Nothing the pending request uses is ever freed: if
//! Windows does not answer, the capture thread gives up and the window is
//! shared without sound. The PC's sound needs no request: the capture
//! thread opens the output itself.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::Duration,
};

pub const RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;
/// Frames per piece sent to the page (20 ms).
pub const CHUNK_FRAMES: usize = 960;
const BYTES_PER_FRAME: usize = 4;

/// Where the sound comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// Only what the app plays.
    #[default]
    App,
    /// Everything the PC plays (the default output).
    Pc,
}

impl Source {
    /// For the diary.
    pub fn label(self) -> &'static str {
        match self {
            Self::App => "the app's sound",
            Self::Pc => "the PC's sound",
        }
    }
}

pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    levels: Arc<Levels>,
}

/// How long the capture thread waits for Windows to answer the request.
const ANSWER_WAIT: Duration = Duration::from_secs(10);

impl Capture {
    /// Starts capturing `source` (call on the main thread) on its own
    /// thread. `on_status` says once whether it works (not if it is stopped
    /// before knowing); `on_chunk` then gets each piece (s16le, interleaved).
    pub fn start(
        source: Source,
        on_chunk: impl Fn(Vec<u8>) + Send + 'static,
        on_status: impl FnOnce(bool) + Send + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let levels = Arc::new(Levels::default());
        let heard = levels.clone();
        crate::diag::note(crate::diag::sound_starting());
        let (sender, receiver) = std::sync::mpsc::channel();
        let requested = match source {
            Source::App => imp::request(sender).map_err(|err| err.to_string()),
            Source::Pc => Ok(()),
        };
        match &requested {
            Ok(()) => crate::diag::note(format!("sound: requested ({})", source.label())),
            Err(err) => crate::diag::note(format!("sound: unavailable ({err})")),
        }
        let thread = std::thread::Builder::new()
            .name("loopback".into())
            .spawn(move || {
                let mut status = Some(on_status);
                let mut report = |ok: bool| {
                    if let Some(status) = status.take() {
                        status(ok);
                    }
                };
                if requested.is_ok() {
                    let result = match source {
                        Source::App => imp::run_app(&flag, receiver, &heard, &on_chunk, &mut report),
                        Source::Pc => imp::run_pc(&flag, &heard, &on_chunk, &mut report),
                    };
                    if let Err(err) = result {
                        crate::diag::note(format!("sound: unavailable ({err})"));
                    }
                }
                // Stopped on purpose before it knew: nothing to say.
                if !flag.load(Ordering::Relaxed) {
                    report(false);
                }
            })
            .ok();
        Self { stop, thread, levels }
    }

    /// What it heard since the last call.
    pub fn take_heard(&self) -> Heard {
        self.levels.take()
    }

    /// Stops it (the thread notices within a fraction of a second).
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What the capture heard in a while: how much sound and how loud.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Heard {
    pub frames: u64,
    /// Loudest sample (0 when everything was silence or nothing came).
    pub peak: u16,
}

/// Shared between the capture thread (which adds) and the app (which takes).
#[derive(Default)]
struct Levels {
    frames: AtomicU64,
    peak: AtomicU32,
}

impl Levels {
    fn add(&self, frames: usize, peak: u16) {
        self.frames.fetch_add(frames as u64, Ordering::Relaxed);
        self.peak.fetch_max(u32::from(peak), Ordering::Relaxed);
    }

    fn take(&self) -> Heard {
        Heard {
            frames: self.frames.swap(0, Ordering::Relaxed),
            peak: u16::try_from(self.peak.swap(0, Ordering::Relaxed)).unwrap_or(u16::MAX),
        }
    }
}

/// The loudest sample of a piece (s16le).
fn loudest(bytes: &[u8]) -> u16 {
    bytes.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs()).max().unwrap_or(0)
}

/// How often the app asks whether the sound is heard.
pub const CHECK_EVERY: Duration = Duration::from_secs(5);
/// Checks in a row with the page playing and nothing heard before saying
/// the capture is deaf (15 s: a short silence in a video is not enough).
const QUIET_CHECKS: u8 = 3;

/// Tells a capture that hears nothing while the page plays sound (the
/// guests would get silence) from a quiet page. Fed every [`CHECK_EVERY`]
/// with what the capture heard and whether WebView2 says the page is
/// audible. Once deaf, it stays so until sound is heard again: a pause
/// does not clear it.
#[derive(Clone, Copy, Debug, Default)]
pub struct SoundWatch {
    quiet: u8,
    deaf: bool,
}

impl SoundWatch {
    /// One check: `Some(deaf)` when that changes.
    pub fn check(&mut self, heard: Heard, page_playing: Option<bool>) -> Option<bool> {
        if heard.peak > 0 {
            self.quiet = 0;
        } else if page_playing == Some(true) {
            self.quiet = self.quiet.saturating_add(1);
        }
        let deaf = self.quiet >= QUIET_CHECKS;
        (deaf != self.deaf).then(|| {
            self.deaf = deaf;
            deaf
        })
    }

    pub fn deaf(&self) -> bool {
        self.deaf
    }
}

/// What the capture got since the last report, for the diary: how much
/// sound arrived, how much of it was silence and how loud it got. Tells a
/// capture that works apart from one that only hears silence.
struct SoundMeter {
    since: std::time::Instant,
    frames: u64,
    silent: u64,
    peak: u16,
}

/// How often the diary gets the sound's numbers.
const REPORT_EVERY: Duration = Duration::from_secs(10);

impl SoundMeter {
    fn new(now: std::time::Instant) -> Self {
        Self { since: now, frames: 0, silent: 0, peak: 0 }
    }

    /// A packet: its samples (s16le), or `None` if Windows marked it silent.
    /// Returns its loudest sample.
    fn packet(&mut self, data: Option<&[u8]>, frames: usize) -> u16 {
        self.frames += frames as u64;
        let peak = data.map_or(0, loudest);
        if peak == 0 {
            self.silent += frames as u64;
        }
        self.peak = self.peak.max(peak);
        peak
    }

    /// The diary's line, every `REPORT_EVERY`.
    fn report(&mut self, now: std::time::Instant) -> Option<String> {
        let elapsed = now.duration_since(self.since);
        if elapsed < REPORT_EVERY {
            return None;
        }
        let heard = self.frames as f64 / f64::from(RATE);
        let line = match (self.silent * 100).checked_div(self.frames) {
            None => format!("sound: nothing in {:.0} s (no packets)", elapsed.as_secs_f64()),
            Some(silent) => format!(
                "sound: {:.1} s in {:.0} s, {}% silent, peak {}%",
                heard,
                elapsed.as_secs_f64(),
                silent,
                u32::from(self.peak) * 100 / 32_768
            ),
        };
        *self = Self::new(now);
        Some(line)
    }
}

/// Collects frames into fixed pieces.
#[derive(Default)]
struct Chunker {
    buffer: Vec<u8>,
}

impl Chunker {
    fn push(&mut self, bytes: &[u8], out: &dyn Fn(Vec<u8>)) {
        self.buffer.extend_from_slice(bytes);
        let size = CHUNK_FRAMES * BYTES_PER_FRAME;
        while self.buffer.len() >= size {
            let rest = self.buffer.split_off(size);
            out(std::mem::replace(&mut self.buffer, rest));
        }
    }

    fn silence(&mut self, frames: usize, out: &dyn Fn(Vec<u8>)) {
        let zeros = vec![0u8; frames * BYTES_PER_FRAME];
        self.push(&zeros, out);
    }
}

/// How the output's own samples are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sample {
    F32,
    S16,
    S24,
    S32,
}

impl Sample {
    fn bytes(self) -> usize {
        match self {
            Self::S16 => 2,
            Self::S24 => 3,
            Self::F32 | Self::S32 => 4,
        }
    }

    fn read(self, b: &[u8]) -> f32 {
        match self {
            Self::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            Self::S16 => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0,
            Self::S24 => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
            Self::S32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        }
    }
}

/// Turns the output's own format into 48 kHz stereo s16le, for outputs that
/// refuse to convert it themselves: surround folded into two channels and
/// the rate changed by linear interpolation (plenty for a shared video).
struct Converter {
    sample: Sample,
    channels: usize,
    /// Input frames per output frame.
    step: f64,
    /// Where the next output frame falls, in input frames from the start of
    /// the next packet (-1 is the last frame of the previous packet).
    pos: f64,
    prev: [f32; 2],
}

impl Converter {
    fn new(sample: Sample, channels: u16, rate: u32) -> Option<Self> {
        (channels > 0 && (8_000..=384_000).contains(&rate)).then(|| Self {
            sample,
            channels: usize::from(channels),
            step: f64::from(rate) / f64::from(RATE),
            pos: 0.0,
            prev: [0.0; 2],
        })
    }

    fn frame_bytes(&self) -> usize {
        self.sample.bytes() * self.channels
    }

    /// One input frame as left and right.
    fn stereo(&self, frame: &[u8]) -> [f32; 2] {
        let size = self.sample.bytes();
        let ch = |i: usize| self.sample.read(&frame[i * size..]);
        match self.channels {
            1 => [ch(0); 2],
            2 => [ch(0), ch(1)],
            n => {
                // Front left/right, the centre at -3 dB to both, no LFE
                // (the fourth), then the rest in left/right pairs at -3 dB.
                const HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;
                let centre = ch(2) * HALF;
                let (mut left, mut right) = (ch(0) + centre, ch(1) + centre);
                for i in 4..n {
                    if i % 2 == 0 {
                        left += ch(i) * HALF;
                    } else {
                        right += ch(i) * HALF;
                    }
                }
                [left, right]
            }
        }
    }

    /// A packet in the output's format (`None`: that many silent frames).
    fn convert(&mut self, data: Option<&[u8]>, frames: usize) -> Vec<u8> {
        let size = self.frame_bytes();
        let input: Vec<[f32; 2]> = match data {
            Some(bytes) => bytes.chunks_exact(size).take(frames).map(|f| self.stereo(f)).collect(),
            None => vec![[0.0; 2]; frames],
        };
        let n = input.len();
        let mut out = Vec::with_capacity(((n as f64 / self.step) as usize + 2) * BYTES_PER_FRAME);
        let last = n as f64 - 1.0;
        while self.pos < last {
            let i = self.pos.floor();
            let frac = (self.pos - i) as f32;
            let a = if i < 0.0 { self.prev } else { input[i as usize] };
            let b = input[(i + 1.0) as usize];
            for c in 0..2 {
                let v = (a[c] + (b[c] - a[c]) * frac).clamp(-1.0, 1.0);
                out.extend_from_slice(&((v * 32_767.0).round() as i16).to_le_bytes());
            }
            self.pos += self.step;
        }
        if let Some(end) = input.last() {
            self.prev = *end;
            self.pos -= n as f64;
        }
        out
    }
}

#[cfg(windows)]
mod imp {
    use std::{
        sync::{atomic::AtomicBool, atomic::Ordering, mpsc, Mutex},
        time::{Duration, Instant},
    };

    use windows::{
        core::{implement, Interface, Ref, Result, HRESULT, PCWSTR},
        Win32::{
            Foundation::{CloseHandle, E_FAIL},
            Media::Audio::{
                eConsole, eRender, ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
                IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
                IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT,
                AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
                AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
                AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVE_FORMAT_PCM,
            },
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                    StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0},
                    BLOB, CLSCTX_ALL, COINIT_MULTITHREADED,
                },
                Threading::{CreateEventW, GetCurrentProcessId, WaitForSingleObject},
                Variant::VT_BLOB,
            },
        },
    };

    use super::{Chunker, Converter, Levels, Sample, SoundMeter, ANSWER_WAIT, BYTES_PER_FRAME, CHANNELS, RATE};

    /// The client handed from Windows' worker thread to the capture thread.
    /// Both are in the multithreaded apartment, where COM objects may be
    /// used from any of its threads.
    pub struct Client(IAudioClient);
    // SAFETY: only moved between threads of the multithreaded apartment
    // (Windows' worker thread → the capture thread, which joins it first).
    unsafe impl Send for Client {}

    pub type Answer = Result<Client>;

    /// Gets the activated client back from Windows' worker thread.
    #[implement(IActivateAudioInterfaceCompletionHandler)]
    struct Activated(Mutex<Option<mpsc::Sender<Answer>>>);

    impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
        fn ActivateCompleted(&self, operation: Ref<IActivateAudioInterfaceAsyncOperation>) -> Result<()> {
            let result = (|| unsafe {
                let operation = operation.ok()?;
                let mut status = HRESULT(0);
                let mut client = None;
                operation.GetActivateResult(&mut status, &mut client)?;
                status.ok()?;
                client.ok_or_else(|| windows::core::Error::from(E_FAIL))?.cast::<IAudioClient>().map(Client)
            })();
            if let Some(sender) = self.0.lock().ok().and_then(|mut s| s.take()) {
                let _ = sender.send(result);
            }
            Ok(())
        }
    }

    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    /// Asks Windows for the app's sound (on the main thread). The answer
    /// goes to `sender`, from a Windows worker thread.
    pub fn request(sender: mpsc::Sender<Answer>) -> Result<()> {
        // Windows may read these until it answers, maybe after we stopped
        // waiting: they are never freed (a few bytes per sharing session).
        let params: &'static AUDIOCLIENT_ACTIVATION_PARAMS = Box::leak(Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: unsafe { GetCurrentProcessId() },
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        }));
        // A PROPVARIANT holding the parameters as a blob (never cleared: it
        // does not own them).
        let variant: &'static PROPVARIANT = Box::leak(Box::new(PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_BLOB,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 {
                        blob: BLOB {
                            cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                            pBlobData: params as *const _ as *mut u8,
                        },
                    },
                }),
            },
        }));
        // Windows keeps the handler until it has answered; releasing our
        // reference to the operation now is allowed.
        let handler: IActivateAudioInterfaceCompletionHandler = Activated(Mutex::new(Some(sender))).into();
        unsafe {
            ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(variant), &handler)?;
        }
        Ok(())
    }

    /// The app's sound: waits for Windows' answer (or gives up), then
    /// captures until `stop`.
    pub fn run_app(
        stop: &AtomicBool,
        answer: mpsc::Receiver<Answer>,
        levels: &Levels,
        on_chunk: &dyn Fn(Vec<u8>),
        report: &mut dyn FnMut(bool),
    ) -> Result<()> {
        // The client comes from Windows' worker thread (multithreaded
        // apartment): this thread joins the same apartment to use it. First,
        // so that it is left last, after the client is released.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        let _com = Com;
        let deadline = Instant::now() + ANSWER_WAIT;
        let client = loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            match answer.recv_timeout(Duration::from_millis(200)) {
                Ok(result) => break result?.0,
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(_) => {
                    return Err(windows::core::Error::new(E_FAIL, "Windows did not answer the request for the app's sound"));
                }
            }
        };
        crate::diag::note("sound: activated");
        // The process loopback has no format of its own: it converts.
        initialize(&client, &our_format(), true)?;
        capture(&client, None, stop, levels, on_chunk, report)
    }

    /// All of the PC's sound: the default output's loopback.
    pub fn run_pc(stop: &AtomicBool, levels: &Levels, on_chunk: &dyn Fn(Vec<u8>), report: &mut dyn FnMut(bool)) -> Result<()> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        let _com = Com;
        let client: IAudioClient = unsafe {
            let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            devices.GetDefaultAudioEndpoint(eRender, eConsole)?.Activate(CLSCTX_ALL, None)?
        };
        crate::diag::note("sound: activated (the PC's output)");
        // Ours if the output converts; otherwise its own, converted here.
        let converter = match initialize(&client, &our_format(), true) {
            Ok(()) => None,
            Err(err) => {
                crate::diag::note(format!("sound: the output does not convert ({err}); converting here"));
                let (format, converter) = mix_format(&client)?;
                initialize(&client, std::ptr::addr_of!(format).cast(), false)?;
                Some(converter)
            }
        };
        capture(&client, converter, stop, levels, on_chunk, report)
    }

    fn our_format() -> WAVEFORMATEX {
        WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: CHANNELS,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * BYTES_PER_FRAME as u32,
            nBlockAlign: BYTES_PER_FRAME as u16,
            wBitsPerSample: 16,
            cbSize: 0,
        }
    }

    fn initialize(client: &IAudioClient, format: *const WAVEFORMATEX, convert: bool) -> Result<()> {
        let mut flags = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        if convert {
            flags |= AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        }
        // 100 ms of buffer (in 100 ns units).
        unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1_000_000, 0, format, None)? };
        crate::diag::note("sound: initialized");
        Ok(())
    }

    /// The output's own format (a copy) and how to convert from it.
    fn mix_format(client: &IAudioClient) -> Result<(WAVEFORMATEXTENSIBLE, Converter)> {
        /// The sub-formats' common tail ({0000xxxx-0000-0010-8000-00AA00389B71}).
        const BASE: u128 = 0x0000_0000_0000_0010_8000_00aa_0038_9b71;
        const KIND: u128 = 0xFFFF_FFFF << 96;
        let unsupported = || windows::core::Error::new(E_FAIL, "the output's format is not one the app knows");
        let (copy, tag) = unsafe {
            let raw = client.GetMixFormat()?;
            let format = *raw;
            // WAVE_FORMAT_EXTENSIBLE: the real kind is in its sub-format.
            let found = if format.wFormatTag == 0xFFFE && format.cbSize >= 22 {
                let full = std::ptr::read_unaligned(raw.cast::<WAVEFORMATEXTENSIBLE>());
                let sub = { full.SubFormat }.to_u128();
                let tag = if sub & !KIND == BASE { (sub >> 96) as u16 } else { 0 };
                (full, tag)
            } else {
                (WAVEFORMATEXTENSIBLE { Format: format, ..Default::default() }, format.wFormatTag)
            };
            CoTaskMemFree(Some(raw as *const _));
            found
        };
        let format = copy.Format;
        let (channels, rate, bits, align) = (format.nChannels, format.nSamplesPerSec, format.wBitsPerSample, format.nBlockAlign);
        let container = align / channels.max(1) * 8;
        let sample = match (tag, bits, container) {
            (3, 32, 32) => Sample::F32,
            (1, 16, 16) => Sample::S16,
            (1, 24, 24) => Sample::S24,
            (1, _, 32) => Sample::S32,
            _ => return Err(unsupported()),
        };
        crate::diag::note(format!("sound: the output's format is {sample:?}, {channels} channels, {rate} Hz"));
        let converter = Converter::new(sample, channels, rate).ok_or_else(unsupported)?;
        Ok((copy, converter))
    }

    /// Captures until `stop`, sending 20 ms pieces.
    fn capture(
        client: &IAudioClient,
        mut converter: Option<Converter>,
        stop: &AtomicBool,
        levels: &Levels,
        on_chunk: &dyn Fn(Vec<u8>),
        report: &mut dyn FnMut(bool),
    ) -> Result<()> {
        let capture: IAudioCaptureClient = unsafe { client.GetService()? };
        let event = unsafe { CreateEventW(None, false, false, PCWSTR::null())? };
        let result = (|| {
            unsafe {
                client.SetEventHandle(event)?;
                client.Start()?;
            }
            crate::diag::note(crate::diag::SOUND_STARTED);
            report(true);
            let mut chunker = Chunker::default();
            let mut meter = SoundMeter::new(Instant::now());
            let frame_bytes = converter.as_ref().map_or(BYTES_PER_FRAME, Converter::frame_bytes);
            while !stop.load(Ordering::Relaxed) {
                if let Some(line) = meter.report(Instant::now()) {
                    crate::diag::note(line);
                }
                // Some outputs never signal a loopback: the packets are also
                // looked for every 10 ms.
                unsafe { WaitForSingleObject(event, 10) };
                loop {
                    let frames = unsafe { capture.GetNextPacketSize()? } as usize;
                    if frames == 0 {
                        break;
                    }
                    let mut data = std::ptr::null_mut();
                    let mut count = 0u32;
                    let mut flags = 0u32;
                    unsafe { capture.GetBuffer(&mut data, &mut count, &mut flags, None, None)? };
                    let count = count as usize;
                    let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null();
                    let raw = (!silent).then(|| unsafe { std::slice::from_raw_parts(data, count * frame_bytes) });
                    match converter.as_mut() {
                        Some(converter) => {
                            let ours = converter.convert(raw, count);
                            let frames = ours.len() / BYTES_PER_FRAME;
                            let peak = meter.packet(raw.map(|_| ours.as_slice()), frames);
                            levels.add(frames, peak);
                            chunker.push(&ours, on_chunk);
                        }
                        None => {
                            let peak = meter.packet(raw, count);
                            levels.add(count, peak);
                            match raw {
                                Some(bytes) => chunker.push(bytes, on_chunk),
                                None => chunker.silence(count, on_chunk),
                            }
                        }
                    }
                    unsafe { capture.ReleaseBuffer(count as u32)? };
                }
            }
            unsafe { client.Stop()? };
            crate::diag::note("sound: stopped");
            Ok(())
        })();
        unsafe {
            let _ = CloseHandle(event);
        }
        result
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::{atomic::AtomicBool, mpsc};

    use super::Levels;

    pub type Answer = Result<(), String>;

    pub fn request(_: mpsc::Sender<Answer>) -> Result<(), String> {
        Err("process loopback needs Windows".into())
    }

    pub fn run_app(
        _: &AtomicBool,
        _: mpsc::Receiver<Answer>,
        _: &Levels,
        _: &dyn Fn(Vec<u8>),
        report: &mut dyn FnMut(bool),
    ) -> Result<(), String> {
        report(false);
        Ok(())
    }

    pub fn run_pc(_: &AtomicBool, _: &Levels, _: &dyn Fn(Vec<u8>), report: &mut dyn FnMut(bool)) -> Result<(), String> {
        report(false);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn the_sound_meter_tells_silence_from_sound() {
        let start = std::time::Instant::now();
        let mut meter = SoundMeter::new(start);
        let mut loud = Vec::new();
        for i in 0..960 {
            let sample = if i == 10 { -16_384i16 } else { 100 };
            loud.extend_from_slice(&sample.to_le_bytes());
            loud.extend_from_slice(&sample.to_le_bytes());
        }
        assert_eq!(meter.packet(Some(&loud), 960), 16_384);
        assert_eq!(meter.packet(None, 960), 0);
        assert_eq!(meter.packet(Some(&[0u8; 960 * 4]), 960), 0);
        meter.packet(Some(&loud), 960);
        assert_eq!(meter.report(start + std::time::Duration::from_secs(3)), None, "not yet");
        let line = meter.report(start + REPORT_EVERY).unwrap();
        assert_eq!(line, "sound: 0.1 s in 10 s, 50% silent, peak 50%");
        assert_eq!(meter.frames, 0, "starts again");
        let line = meter.report(start + REPORT_EVERY * 2).unwrap();
        assert_eq!(line, "sound: nothing in 10 s (no packets)");
    }

    #[test]
    fn pieces_are_twenty_milliseconds() {
        let out = RefCell::new(Vec::new());
        let sink = |piece: Vec<u8>| out.borrow_mut().push(piece);
        let mut chunker = Chunker::default();
        chunker.push(&vec![1u8; 500 * BYTES_PER_FRAME], &sink);
        assert!(out.borrow().is_empty());
        chunker.silence(1_500, &sink);
        let pieces = out.borrow();
        assert_eq!(pieces.len(), 2);
        assert!(pieces.iter().all(|p| p.len() == CHUNK_FRAMES * BYTES_PER_FRAME));
        assert_eq!(pieces[0][0], 1);
        assert_eq!(pieces[1][0], 0);
        assert_eq!(chunker.buffer.len(), (2_000 - 2 * CHUNK_FRAMES) * BYTES_PER_FRAME);
        assert_eq!(RATE as usize * 20 / 1000, CHUNK_FRAMES);
    }

    #[test]
    fn levels_are_taken_and_start_again() {
        let levels = Levels::default();
        levels.add(960, 100);
        levels.add(960, 3_000);
        levels.add(960, 0);
        assert_eq!(levels.take(), Heard { frames: 2_880, peak: 3_000 });
        assert_eq!(levels.take(), Heard::default());
    }

    const LOUD: Heard = Heard { frames: 240_000, peak: 9_000 };
    const QUIET: Heard = Heard { frames: 240_000, peak: 0 };
    const NOTHING: Heard = Heard { frames: 0, peak: 0 };

    #[test]
    fn a_deaf_capture_is_noticed_after_fifteen_seconds_of_playing() {
        let mut watch = SoundWatch::default();
        assert_eq!(watch.check(QUIET, Some(true)), None);
        assert_eq!(watch.check(NOTHING, Some(true)), None);
        assert_eq!(watch.check(QUIET, Some(true)), Some(true), "three checks (15 s) of silence while playing");
        assert!(watch.deaf());
        assert_eq!(watch.check(QUIET, Some(true)), None, "said once");
        // Paused: still deaf (nothing proves otherwise).
        assert_eq!(watch.check(NOTHING, Some(false)), None);
        assert_eq!(watch.check(LOUD, Some(true)), Some(false), "sound again");
        assert!(!watch.deaf());
    }

    #[test]
    fn a_quiet_page_or_short_silences_are_not_deafness() {
        let mut watch = SoundWatch::default();
        for _ in 0..20 {
            assert_eq!(watch.check(NOTHING, Some(false)), None, "nothing plays: nothing to hear");
            assert_eq!(watch.check(QUIET, None), None, "WebView2 can't say: no guess");
        }
        // A silent passage in a video: two quiet checks, then sound.
        for _ in 0..10 {
            assert_eq!(watch.check(QUIET, Some(true)), None);
            assert_eq!(watch.check(QUIET, Some(true)), None);
            assert_eq!(watch.check(LOUD, Some(true)), None);
        }
        assert_eq!(CHECK_EVERY * u32::from(QUIET_CHECKS), Duration::from_secs(15));
    }

    fn floats(frames: &[&[f32]]) -> Vec<u8> {
        frames.iter().flat_map(|f| f.iter().flat_map(|s| s.to_le_bytes())).collect()
    }

    fn samples(bytes: &[u8]) -> Vec<i16> {
        bytes.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]])).collect()
    }

    #[test]
    fn float_stereo_at_48k_only_changes_format() {
        let mut converter = Converter::new(Sample::F32, 2, 48_000).unwrap();
        assert_eq!(converter.frame_bytes(), 8);
        let input = floats(&[&[0.5, -0.5], &[1.0, -1.0], &[2.0, 0.0], &[0.0, 0.25]]);
        let mut out = converter.convert(Some(&input), 4);
        // The last frame waits for the next packet (it is interpolated with it).
        out.extend(converter.convert(Some(&floats(&[&[0.0, 0.0]])), 1));
        assert_eq!(samples(&out), vec![16_384, -16_384, 32_767, -32_767, 32_767, 0, 0, 8_192]);
    }

    #[test]
    fn rates_change_without_drift_across_packets() {
        for rate in [44_100u32, 96_000, 16_000] {
            let mut converter = Converter::new(Sample::S16, 1, rate).unwrap();
            let packet: Vec<u8> = (0..441).flat_map(|i| ((i % 100) as i16 * 100).to_le_bytes()).collect();
            let mut frames = 0usize;
            for _ in 0..1_000 {
                frames += converter.convert(Some(&packet), 441).len() / BYTES_PER_FRAME;
            }
            // 441 000 input frames at `rate` → this many at 48 kHz, but for
            // the frames after the last input one (they wait for the next
            // packet): no drift after 1 000 packets.
            let expected = 441_000f64 * 48_000.0 / f64::from(rate);
            let waiting = 48_000.0 / f64::from(rate) + 1.0;
            assert!((frames as f64 - expected).abs() <= waiting, "{rate}: {frames} vs {expected}");
        }
    }

    #[test]
    fn interpolation_is_linear_and_continues_over_packets() {
        // 24 kHz → 48 kHz: every other output frame lies halfway.
        let mut converter = Converter::new(Sample::S16, 2, 24_000).unwrap();
        let frame = |v: i16| [v.to_le_bytes(), v.to_le_bytes()].concat();
        let first: Vec<u8> = [0i16, 1_000].iter().flat_map(|v| frame(*v)).collect();
        let second: Vec<u8> = [2_000i16, 3_000].iter().flat_map(|v| frame(*v)).collect();
        let mut out = samples(&converter.convert(Some(&first), 2));
        out.extend(samples(&converter.convert(Some(&second), 2)));
        let left: Vec<i16> = out.iter().step_by(2).copied().collect();
        assert_eq!(left, vec![0, 500, 1_000, 1_500, 2_000, 2_500]);
    }

    #[test]
    fn surround_folds_into_two_channels_and_mono_doubles() {
        // 5.1: FL FR FC LFE BL BR.
        let mut converter = Converter::new(Sample::F32, 6, 48_000).unwrap();
        let six = floats(&[&[0.1, 0.0, 0.2, 0.9, 0.3, 0.0], &[0.0; 6]]);
        let out = samples(&converter.convert(Some(&six), 2));
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let left = ((0.1 + 0.2 * half + 0.3 * half) * 32_767.0f32).round() as i16;
        let right = ((0.2 * half) * 32_767.0f32).round() as i16;
        assert_eq!(out, vec![left, right], "no LFE, centre to both, back to its side");
        let mut mono = Converter::new(Sample::S24, 1, 48_000).unwrap();
        let input = [[0x00u8, 0x00, 0x40], [0x00, 0x00, 0xC0]].concat();
        assert_eq!(samples(&mono.convert(Some(&input), 2)), vec![16_384, 16_384]);
        let mut s32 = Converter::new(Sample::S32, 2, 48_000).unwrap();
        let input: Vec<u8> = [i32::MIN, 1 << 30, 0, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(samples(&s32.convert(Some(&input), 2)), vec![-32_767, 16_384]);
    }

    #[test]
    fn silent_packets_convert_to_silence_and_odd_formats_are_refused() {
        let mut converter = Converter::new(Sample::F32, 2, 44_100).unwrap();
        let out = converter.convert(None, 441);
        assert!(!out.is_empty() && out.iter().all(|b| *b == 0));
        assert!(Converter::new(Sample::F32, 0, 48_000).is_none());
        assert!(Converter::new(Sample::F32, 2, 1_000).is_none());
        assert!(Converter::new(Sample::F32, 2, 48_000).unwrap().convert(Some(&[]), 0).is_empty());
    }
}

#[cfg(test)]
#[path = "tests/loopback_cases.rs"]
mod cases;
