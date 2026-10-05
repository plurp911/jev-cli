# Install jev

Download the archive for your operating system and CPU from the
[v0.3.0 release](https://github.com/plurp911/jev-cli/releases/tag/v0.3.0).
The release provides these builds:

| System | Archive |
| --- | --- |
| Linux x86-64 | `jev-cli-x86_64-unknown-linux-gnu.tar.xz` |
| Linux ARM64 | `jev-cli-aarch64-unknown-linux-gnu.tar.xz` |
| macOS Intel | `jev-cli-x86_64-apple-darwin.tar.xz` |
| macOS Apple silicon | `jev-cli-aarch64-apple-darwin.tar.xz` |
| Windows x86-64 | `jev-cli-x86_64-pc-windows-msvc.zip` |

These archives install the CLI. Local inference also needs a separately installed
server, compatible model weights, and suitable hardware; archive availability does
not establish native model inference on every operating system.

Each archive has an adjacent `.sha256` file. Download both files and check the digest
before extracting the archive. See [Verifying a release](release-verification.md) for
the aggregate checksum and build provenance checks.

## Linux and macOS

Replace `archive` with the name in the table above. On Linux, run:

```sh
base=https://github.com/plurp911/jev-cli/releases/download/v0.3.0
archive=jev-cli-x86_64-unknown-linux-gnu.tar.xz
curl -fLO "$base/$archive" -fLO "$base/$archive.sha256"
sha256sum --check "$archive.sha256"
tar -xJf "$archive"
directory=${archive%.tar.xz}
"./$directory/jev" doctor
```

On macOS, choose the matching macOS archive and replace the checksum command with
`sed -n '1p' "$archive.sha256" | shasum -a 256 --check`. The binary has no Apple
notarization; macOS may ask you to approve it before it runs.

## Windows

Download the Windows `.zip` and its `.zip.sha256` file from the release page. In
PowerShell, run these commands in the download directory:

```powershell
$archive = 'jev-cli-x86_64-pc-windows-msvc.zip'
$expected = (Get-Content "$archive.sha256" -Raw).Split(' ')[0]
$actual = (Get-FileHash $archive -Algorithm SHA256).Hash
if ($actual -ne $expected) { throw 'Archive checksum mismatch' }
Expand-Archive $archive -DestinationPath .
& '.\jev-cli-x86_64-pc-windows-msvc\jev.exe' doctor
```

The Windows binary has no code signature, so SmartScreen may show a warning. Read
[Verifying a release](release-verification.md) before deciding whether to run it.

## Choose a provider and ask a question

TypeSafe is the default provider. For its Jev model:

Get an API key from TypeSafe. On a machine with an OS credential store, run `jev auth
login` and enter the key at the hidden prompt. For a headless environment, set
`JEV_API_KEY_FILE` to a file supplied by your secret manager. See the
[authentication guide](../README.md#authenticate) for other supported options.

Run `jev doctor` to check the configuration without contacting the API. Then try the
[first question](../README.md#first-question). The state you give to a question is
sent to TypeSafe.

For hosted Clef or Clef Flash, select `--provider cloudflare`, supply your account ID
through `CLOUDFLARE_ACCOUNT_ID` or `--cloudflare-account-id`, and supply a Workers AI
token through `JEV_CUSTOM_API_KEY` or `JEV_CUSTOM_API_KEY_FILE`. `jev auth login`
stores a TypeSafe credential; it does not configure Cloudflare.

For local Clef, explicitly choose `--provider ollama`, `--provider llamacpp`, or
`--provider huggingface`. Start the compatible server and obtain its weights
separately. The Python bridge is a source-repository script, not a Python runtime
embedded in the release binary. Loopback local providers need no credential.
The CLI never launches servers, downloads weights, or falls back to hosted inference.

See [Clef setup and capabilities](clef.md) for hosted/local setup, image commands,
prepared video frames, and the tested configurations. Check the selected setup
without an inference using, for example, `jev --provider ollama doctor`; add `--live`
to query the server's model-listing endpoint.

## Build from source

If there is no archive for your platform, build with Rust 1.88 or newer:

```sh
git clone https://github.com/plurp911/jev-cli
cd jev-cli
cargo build --release --locked
./target/release/jev doctor
```
