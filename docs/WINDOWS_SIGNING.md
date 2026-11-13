# Windows code signing (Azure Artifact Signing)

Windows releases are Authenticode-signed during `tauri build` on the
`windows-latest` job of [`release.yml`](../.github/workflows/release.yml).
The certificate is issued by Microsoft's public CA to the identity-validated
publisher (**GoEncode LLC**) and lives in Microsoft's HSM: there is no `.pfx`
anywhere, not in the repository, not in a secret, not on a laptop. Signing
happens only inside the release workflow, only on a pushed version tag.

What gets signed (Tauri's bundler applies `bundle.windows.signCommand` to
each): the app executable, the `hook-bridge` sidecar, the NSIS `-setup.exe`,
its uninstaller, the NSIS plugin DLLs and the MSI. The updater's own
`.sig` files (minisign, `TAURI_SIGNING_PRIVATE_KEY`) are a separate layer
and unchanged.

Until the repository variables below exist, the step is skipped and the
Windows build ships unsigned exactly as before. Forks never sign.

## One-time set-up in Azure (maintainer)

Microsoft's walkthrough: <https://learn.microsoft.com/azure/artifact-signing/quickstart>.

1. **Subscription and resource group.** Any region; the signing endpoint is
   regional (for example `https://eus.codesigning.azure.net`, `wus2`, `neu`).
2. **Artifact Signing account** (Basic tier is enough for one app).
3. **Identity validation**, type *Organization*, for the legal entity that
   will appear as the publisher. The validated name is the certificate's
   subject (CN) and must equal `bundle.publisher` in
   [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json) — today
   `GoEncode LLC`. Microsoft asks for a registration number and a contact
   it can reach; the organization needs a verifiable history (three years
   as of 2026). Allow a few business days.
4. **Certificate profile**, type *Public Trust*, bound to that identity.
   Note its name.
5. **An Entra app registration** for the workflow (client credentials):
   create the app, add a client secret, and on the *Artifact Signing
   account* grant the app the role **Artifact Signing Certificate Profile
   Signer** (IAM → Add role assignment). Nothing else; it cannot read or
   export the key.

## Configure the repository

Repository **variables** (Settings → Secrets and variables → Actions →
Variables), all three required for the step to run:

| Variable                 | Value                                   |
| ------------------------ | --------------------------------------- |
| `AZURE_SIGNING_ENDPOINT` | the account's regional endpoint URL     |
| `AZURE_SIGNING_ACCOUNT`  | the Artifact Signing account name       |
| `AZURE_SIGNING_PROFILE`  | the certificate profile name            |

Repository **secrets**:

| Secret                | Value                                       |
| --------------------- | ------------------------------------------- |
| `AZURE_TENANT_ID`     | the Entra tenant id                         |
| `AZURE_CLIENT_ID`     | the app registration's application id       |
| `AZURE_CLIENT_SECRET` | the app registration's client secret value  |

Rotate the client secret before it expires (Entra caps them at 24 months);
the workflow fails loudly, not silently, when it does.

## How the workflow signs

- `cargo install artifact-signing-cli --locked --version 0.12.0` — the CLI
  (<https://github.com/levminer/trusted-signing-cli>) wraps `signtool` with
  Microsoft's Artifact Signing dlib and authenticates with the `AZURE_*`
  secrets. The version is pinned; bump it on purpose.
- The step writes `src-tauri/azure-signing.conf.json` (git-ignored) holding
  `bundle.windows.signCommand` in Tauri's object form, so the description
  `Agent Console` survives as one argument, and passes it to `tauri build`
  with `--config`. The committed `tauri.conf.json` never carries the sign
  command: local and fork builds are unaffected.
- The MSI's `-d` description is what the UAC prompt shows as the installer
  name.

## Verify a release

On Windows, for the `-setup.exe`, the `.msi` and the unpacked
`agent-console.exe` and `hook-bridge.exe`:

```powershell
signtool verify /pa /v .\agent-console_X.Y.Z_x64-setup.exe
Get-AuthenticodeSignature .\agent-console_X.Y.Z_x64-setup.exe | Format-List
```

Expect `Status: Valid`, the subject `CN=GoEncode LLC, ...`, an issuer under
*Microsoft Identity Verification Root Certificate Authority 2020*, and a
timestamp (Artifact Signing timestamps automatically; the signature stays
valid after the short-lived certificate expires).

## SmartScreen and antivirus after the first signed release

- A valid signature removes "Unknown publisher" at once. SmartScreen's
  "Windows protected your PC" is reputation, earned per publisher identity
  over downloads; expect it to fade over the first weeks. Keep the publisher
  name and the certificate profile stable: a new identity starts over.
- Before announcing a release, upload the installer to VirusTotal. If a
  vendor flags it, use its false-positive form (Microsoft:
  <https://www.microsoft.com/wdsi/filesubmission>); what trips heuristics
  here is legitimate behaviour — spawning shells, writing `~/.claude`
  hooks, a TCP listener for paired phones, a self-updater.
- Windows Defender Firewall still asks once when the user enables *Remote
  devices* on the LAN: the app binds a LAN address only then, and the
  prompt is the right behaviour. The installer does not pre-register a
  firewall rule on purpose.
