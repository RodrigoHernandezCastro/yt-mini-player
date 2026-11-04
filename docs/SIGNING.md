# Signing the releases (Authenticode)

Releases are built on GitHub (`release.yml`) with SHA-256 fingerprints and a provenance
attestation, but until a signing certificate is set up the `.exe` files carry no Authenticode
signature. Without one, Windows SmartScreen says "Unknown publisher" and antivirus programs are
quicker to hold the files back (Microsoft Defender quarantined `install.ps1` in 0.10.0). This page
explains what signing changes, which certificates fit this project and how to switch it on. The
build already does the rest: `tools/sign.ps1` signs both programs and verifies the result,
`tools/package.ps1` calls it before writing `SHA256SUMS.txt`, and the release fails if signing was
set up but did not work.

Prices and eligibility were checked in November 2026 and change often: confirm them before
paying.

## What a signature does, and what it does not

- Windows shows the publisher's verified name instead of "Unknown publisher", and nobody can change
  a signed file without breaking the signature.
- SmartScreen still learns from downloads. Microsoft's own guidance: a newly signed file can show
  the warning until its hash or the certificate builds reputation, which "can take several weeks and
  hundreds of clean installs". Extended Validation (EV) certificates **no longer** skip this, so
  paying for EV only to avoid SmartScreen is not worth it
  ([Microsoft Learn: SmartScreen reputation](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)).
  Keeping the same certificate from release to release is what builds that reputation.
- The maximum overlay install no longer needs its local certificate: `install.ps1 -Overlay` sees
  that `yt-mini-player-uia.exe` already has a valid publisher signature and skips creating,
  trusting and deleting the local one (that step is also the kind of thing antivirus programs
  watch for).

## Which certificate

| Option | Who can get it | Cost | Fits this project? |
|---|---|---|---|
| [Azure Artifact Signing](https://azure.microsoft.com/products/trusted-signing) (formerly Trusted Signing) | At general availability (January 2026): individuals in the USA and Canada; organizations in the USA, Canada, the EU and the UK | Basic USD 9.99/month (5,000 signatures) | Best for CI: signs inside `release.yml`, no key to look after. **Not open to individuals in Chile** at the time of writing; check the current regions |
| [Certum Standard Code Signing in the Cloud](https://shop.certum.eu/code-signing.html) | Individuals and companies worldwide (identity check) | About EUR 209 (cloud, "SimplySign"); EUR 169 with card and reader | Works today for an individual. The key stays in Certum's cloud and is unlocked with a phone app, so signing happens on your PC (see below) |
| Certum Open Source Code Signing | Developers of open-source software | About EUR 49 | No: the app has paid plans and no open-source licence |
| [SignPath Foundation](https://signpath.org/terms) | OSI-licensed projects with no proprietary parts, no commercial dual licensing | Free; the publisher shown is "SignPath Foundation" | No, for the same reason |
| Self-signed certificate | Anyone | Free | No: Windows does not trust it, so SmartScreen is unchanged |

Since 27 February 2026 a code signing certificate may last at most 459 days, so multi-year
purchases come with free re-issues (Certum's shop says so). Timestamps keep old signatures valid
after a certificate expires; `tools/sign.ps1` refuses a signature without one.

## Signing in CI (Azure Artifact Signing)

1. In Azure, create an Artifact Signing account, pass the identity validation and create a
   *Public Trust* certificate profile. Note the account's endpoint (it depends on the region,
   for example `https://eus.codesigning.azure.net`; a wrong region gives "403 Forbidden").
2. Create an app registration with a **federated credential for GitHub Actions** limited to this
   repository's tags (`repo:<owner>/<repo>:ref:refs/tags/*`), and give it the certificate
   profile's signer role (*Artifact Signing Certificate Profile Signer*; older documents call it
   *Trusted Signing Certificate Profile Signer*). No client secret is needed.
3. In the repository's **Settings → Secrets and variables → Actions → Variables**, add:

   | Variable | Value |
   |---|---|
   | `AZURE_CLIENT_ID` | the app registration's client ID |
   | `AZURE_TENANT_ID` | the tenant ID |
   | `AZURE_SUBSCRIPTION_ID` | the subscription with the signing account |
   | `AZURE_SIGNING_ENDPOINT` | the endpoint from step 1 |
   | `AZURE_SIGNING_ACCOUNT` | the signing account's name |
   | `AZURE_SIGNING_PROFILE` | the certificate profile's name |

4. Push a `v*` tag. `release.yml` builds and tests in a job that cannot sign (read-only
   rights: no dependency's build code ever runs where signing is possible), then a second job
   that builds nothing takes the two programs, signs in with OIDC (`azure/login`), downloads
   the `Microsoft.ArtifactSigning.Client` package (a fixed version), writes its
   `metadata.json` and runs the package (`tools/package.ps1 -Prebuilt`):
   both programs are signed with `signtool /dlib`, timestamped by `timestamp.acs.microsoft.com`
   and verified. The job summary lists each file's signature and signer, and the provenance
   attestation covers the **signed** files.

If the variables are missing the release is built unsigned, exactly as before, with a warning in
the log.

## Signing on your PC (a certificate in the certificate store)

For Certum SimplySign, a hardware token or an organization's certificate: anything that puts the
certificate in Windows' certificate store.

1. Install the Windows SDK's signing tools (they bring `signtool.exe`), and for Certum, SimplySign
   Desktop; connect it with the code from the phone app.
2. Find the certificate's thumbprint:
   ```powershell
   Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert | Format-List Subject, Thumbprint, NotAfter
   ```
3. Build and sign:
   ```powershell
   $env:YTMP_SIGN_THUMBPRINT = '<40 hex characters>'
   $env:YTMP_SIGN_TIMESTAMP  = 'http://time.certum.pl'   # or your CA's RFC 3161 server
   $env:YTMP_SIGN_REQUIRED   = '1'                       # fail instead of packaging unsigned
   .\tools\package.ps1
   ```
4. Upload the zip and its SHA-256 to the release by hand.

The trade-off: GitHub's provenance attestation only covers files built by `release.yml`, so a
package built on your PC has none. Say in the release notes that it was built and signed locally,
or prefer CI signing once it is available to you.

## When an antivirus still holds a file back

- The package carries `ANTIVIRUS.txt`. If `install.ps1` or a program is missing after unpacking,
  `Install.cmd` (and the other two launchers) show its section in the Windows display language:
  check the zip's SHA-256, restore the file from Windows Security's *Protection history* or unpack
  again. `install.ps1` points to it too when a program is missing.
- Report false positives from the publisher's side as well: Microsoft takes submissions at
  [microsoft.com/wdsi/filesubmission](https://www.microsoft.com/wdsi/filesubmission) (choose
  *Software developer*; it needs a Microsoft account). That portal is for Defender detections;
  SmartScreen reputation has no submission process and only builds with downloads. Most other
  antivirus vendors have a false-positive form too.

## Checking a release

```powershell
Get-AuthenticodeSignature .\yt-mini-player.exe | Format-List Status, SignerCertificate, TimeStamperCertificate
signtool verify /pa /v .\yt-mini-player.exe
gh attestation verify YT-Mini-Player-<version>-win64.zip --repo <owner>/<repo>
```
