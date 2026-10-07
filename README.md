# YT Mini Player

Reproductor flotante de YouTube para Windows 10/11: ventana sin bordes, siempre visible,
sesión persistente. Rust + [tao](https://crates.io/crates/tao) + [wry](https://crates.io/crates/wry) (WebView2).

Versión 0.2.0.

## Instalar (usuario final)

Descomprime `YT-Mini-Player-0.2.0-win64.zip` y haz doble clic en **Instalar.cmd**. Copia la app a
`%LOCALAPPDATA%\Programs\YT Mini Player`, crea accesos directos en el menú Inicio y el escritorio,
y la abre. No pide administrador. `Desinstalar.cmd` la quita (conserva la sesión).

El `.exe` no está firmado: la primera vez SmartScreen puede mostrar "Windows protegió su PC" →
"Más información" → "Ejecutar de todas formas".

## Uso

| Acción | Cómo |
| --- | --- |
| Mover | Arrastrar el video · Alt + arrastrar en cualquier punto · asa ⠿ de la barra |
| Redimensionar | Arrastrar un borde (6 px) o una esquina (12 px) |
| Barra de botones | Acercar el ratón al borde derecho |
| Más opciones | Botón ⋯ de la barra |
| Ocultar | Botón del ojo, Ctrl+H, o Ctrl+Alt+Y desde cualquier sitio |
| Cerrar | Botón ×, Ctrl+Q, Alt+F4 o la bandeja → Salir |

Un clic sin mover sobre el video sigue pausando y reproduciendo, como siempre.

**Barra lateral:** mover, atrás, solo video, pantalla completa, siempre visible, ocultar, más, cerrar.
Con la ventana baja (< 300 px) solo quedan los esenciales; el resto sigue en el menú ⋯.

**Menú ⋯:** tamaños S/M/L, esquinas, opacidad, modo fantasma, inicio de YouTube, abrir enlace
copiado, extensiones, iniciar con Windows, atajos y salir.

**Bandeja del sistema:** clic izquierdo muestra u oculta; clic derecho abre un menú con siempre
visible, modo fantasma, opacidad al 100 %, iniciar con Windows, carpeta de extensiones y salir.

**Instancia única:** abrir el programa estando ya abierto (o escondido) lo trae al frente.

### Atajos

Con la ventana activa (F1 los muestra en pantalla):

| Atajo | Acción |
| --- | --- |
| F11 | Pantalla completa |
| Ctrl+T | Siempre visible |
| Ctrl+M | Solo video |
| Ctrl+H | Ocultar en la bandeja |
| Ctrl+L | Abrir el enlace de YouTube copiado |
| Ctrl+1 / 2 / 3 | Tamaño pequeño / mediano / grande |
| Ctrl+Shift+1 … 4 | Esquina ↖ ↗ ↙ ↘ |
| Alt+Inicio | Inicio de YouTube |
| Ctrl+Q | Salir |

En todo Windows, aunque esté oculto:

| Atajo | Acción |
| --- | --- |
| Ctrl+Alt+Y | Mostrar u ocultar |
| Ctrl+Alt+P | Reproducir / pausar |
| Ctrl+Alt+N / B | Siguiente / anterior |
| Ctrl+Alt+G | Modo fantasma (los clics atraviesan la ventana) |
| Ctrl+Alt+RePág / AvPág | Más / menos opacidad |
| Ctrl+Alt+0 | Opacidad al 100 % |

Se usan Ctrl+Alt + letras que no escriben símbolos con AltGr en las distribuciones española y
latinoamericana, y se evitan Ctrl+Alt+flechas (algunos controladores Intel giran la pantalla).
Si otro programa ya usa un atajo, la ayuda lo muestra tachado. Los atajos de YouTube (espacio,
K, J, L, F, M…) siguen funcionando.

### Extensiones de Chrome

Menú ⋯ → Extensiones → **Abrir carpeta** (`%LOCALAPPDATA%\yt_mini_player\extensions`). Copia ahí:

- carpetas descomprimidas (con `manifest.json`, también si vienen dentro de una carpeta envolvente),
- archivos `.crx` (versiones 2 y 3) o `.zip`: se descomprimen solos y el original se aparta a
  `.archivos\`. Un `.crx` con el mismo nombre actualiza la extensión.

Pulsa **Recargar** y luego F5. Cada extensión se puede activar o desactivar y, si tiene página de
opciones, abrirla. Si borras su carpeta, se desinstala al recargar.

Limitaciones de WebView2: no hay Chrome Web Store, ni barra de extensiones, ni sus ventanas
emergentes; las extensiones Manifest V2 probablemente no carguen. Requiere un runtime de WebView2
reciente (si no lo es, el menú lo indica).

## Compilar

Requisitos: Rust estable y las **Build Tools de Visual Studio** con "Desarrollo para el escritorio
con C++" (VS Code no basta: falta `link.exe`).

```powershell
cargo run                                                 # debug, con consola para ver logs
powershell -ExecutionPolicy Bypass -File tools\package.ps1 # release + dist\YT-Mini-Player-<v>-win64.zip
```

Con MSVC el cargador de WebView2 queda dentro del `.exe`. El paquete precompilado se generó desde
Linux con MinGW (target GNU) y por eso lleva `WebView2Loader.dll` al lado, que es redistribuible.

## Estructura

| Archivo | Contenido |
| --- | --- |
| `src/main.rs` | Entrada, instancia única, bucle de eventos |
| `src/app.rs` | Estado de la app y ejecución de todas las órdenes |
| `src/ipc.rs` | Protocolo página → host (lista cerrada) y órdenes internas |
| `src/window.rs` | Ventana sin bordes, icono, geometría restaurada |
| `src/webview.rs` | WebView2 con perfil aislado, scripts inyectados, extensiones activadas |
| `src/extensions.rs` | `.crx`/`.zip`, manifest, sincronización con el perfil de WebView2 |
| `src/tray.rs` | Icono y menú de la bandeja |
| `src/hotkeys.rs` | Atajos globales |
| `src/platform.rs` | Win32: opacidad, clic a través, portapapeles, esquinas, inicio con Windows |
| `src/links.rs` | Validación de enlaces de YouTube del portapapeles |
| `src/config.rs` | Rutas, `settings.json`, `window.json`, `extensions.json` |
| `src/lifecycle.rs` | Menos memoria y render al minimizar u ocultar |
| `src/inject.js` | Arrastrar el video, Alt+arrastrar, bordes |
| `src/ui.js` | Barra lateral, menú, ayuda, avisos, atajos locales, solo video |
| `src/maintenance.js` | Recarga preventiva en sesiones largas |
| `dist/` | Instalador (`install.ps1`, `Instalar.cmd`, `Desinstalar.cmd`, `LEEME.txt`) |
| `tools/` | `measure.ps1` (memoria y CPU), `package.ps1` (empaquetar) |

Datos del usuario en `%LOCALAPPDATA%\yt_mini_player`: `profile\` (sesión), `extensions\`,
`settings.json`, `window.json`, `extensions.json`.

## Pruebas

```powershell
cargo test                               # protocolo, enlaces, extensiones, ajustes, esquinas
node tests/inject.test.js                # arrastre y bordes, DOM simulado
node tests/maintenance.test.js           # recarga preventiva, reloj simulado
python tests/ui_e2e.py capturas          # interfaz completa en Chromium (pip install playwright)
powershell -ExecutionPolicy Bypass -File tests\measure.test.ps1
```

`ui_e2e.py` carga una página que imita el reproductor de YouTube con la política Trusted Types
real y comprueba 40 comportamientos: barra, menú, arrastre sin pausar, atajos, solo video,
pantalla completa y ausencia de errores.

Pruebas manuales pendientes en Windows real:

- [ ] Arrastrar el video mueve la ventana y no lo pausa ni lo deja a 2x.
- [ ] Ocultar con el ojo y volver con Ctrl+Alt+Y o la bandeja; el audio sigue sonando oculto.
- [ ] Pantalla completa (F11 o el botón de YouTube) ocupa el monitor; Esc vuelve al tamaño anterior.
- [ ] Opacidad 70 % se ve translúcida (no negra); modo fantasma deja pasar los clics.
- [ ] Una extensión `.crx` aparece en el menú y funciona tras F5.
- [ ] Iniciar con Windows arranca oculto en la bandeja.

## Problemas conocidos

- Google puede bloquear el inicio de sesión en webviews pese al User-Agent (sección 8.1 de la propuesta).
- **Opacidad y modo fantasma son experimentales:** usan una ventana *layered* de Win32 y no está
  verificado cómo se combina con el render de WebView2. Si la ventana se ve negra, Ctrl+Alt+0 o la
  bandeja → "Opacidad al 100 %" lo restablecen.
- Solo video y la detección de "superficie de video" dependen de la estructura de la página de
  YouTube; si YouTube la cambia, puede requerir ajustes en `ui.js` / `inject.js`.
- La página de opciones de una extensión se abre en la misma ventana (Alt+← para volver).
- La franja derecha de redimensionado se solapa con parte de la barra de desplazamiento.
- La recarga preventiva pierde la cola temporal ("Añadir a la cola").
