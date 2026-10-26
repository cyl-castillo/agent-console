# Teams sign-in without configuration: the app's own public client

Since v0.94 the Teams panel can sign a user in with one button. For that the
build must carry the Application (client) ID of an Entra app registration
owned by the project — a *public client*, which has no secret, so shipping
its id is the same model as the Azure CLI or VS Code. Until the id is set,
the panel asks each user for their own registration, as before.

## One-time registration (project owner)

1. <https://entra.microsoft.com> → Identity → Applications → App registrations
   → **New registration**.
   - Name: `Agent Console`.
   - Supported account types: **Accounts in any organizational directory
     (Any Microsoft Entra ID tenant – Multitenant)**.
   - Redirect URI: leave empty (device code flow needs none).
2. In the new app → **Authentication** → *Advanced settings* →
   **Allow public client flows: Yes** → Save.
3. **API permissions** → Add a permission → Microsoft Graph → *Delegated* →
   `User.Read` and `Chat.Read` (what `teams_service::SCOPES` requests). No
   admin consent is granted here: each tenant decides.
4. **Overview** → copy the **Application (client) ID**.
5. GitHub → repository → Settings → Secrets and variables → Actions →
   **Variables** → `AC_TEAMS_CLIENT_ID` = that id. (A variable, not a secret:
   the id is public by design and is compiled into the binary.)

The next release builds with it (`release.yml` passes the variable to the
build; `build.rs` re-runs when it changes). A local `tauri build` can set
`AC_TEAMS_CLIENT_ID=…` in the environment to the same effect.

## What users see

- With the id: **Sign in with Microsoft** → the device code screen → done.
  Tokens stay in the OS keychain, as before.
- Tenants that block third-party apps: the same screen offers *Use your own
  app registration*, which is the old form (client id + tenant).
- Expect an admin-consent prompt the first time in tenants that require it;
  that is the tenant's policy, not something the app can skip.
