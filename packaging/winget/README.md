# WinGet

WinGet manifests live in [`microsoft/winget-pkgs`][repo], not here. These are the
templates a human fills in and submits as a pull request there, after verifying the
published artifacts.

Three files go in
`manifests/p/plurp911/jev/<version>/`, all with `PackageIdentifier: plurp911.jev`:

- `plurp911.jev.yaml` — the version manifest
- `plurp911.jev.installer.yaml` — the installer manifest, carrying the URL and SHA-256
- `plurp911.jev.locale.en-US.yaml` — the default-locale manifest

Validate before submitting:

```powershell
winget validate --manifest manifests\p\plurp911\jev\<version>
winget install --manifest manifests\p\plurp911\jev\<version>
```

The `ShortDescription` must keep the "unofficial, community" wording. A package listing
that reads as though TypeSafe published it would be a misrepresentation, and the store
listing is exactly where that misreading is most likely.

[repo]: https://github.com/microsoft/winget-pkgs
