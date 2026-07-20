# AUR: agent-console-bin

Status: **prepared — first publish pending the owner's AUR credentials.**

The package ([packaging/aur/PKGBUILD](../packaging/aur/PKGBUILD)) repacks the
released `.deb` — identical binaries to every channel. The `publish-aur` job
in release.yml pushes it to the AUR on every release (version from the tag,
checksums recomputed via updpkgsums in an Arch container). The very first
push also *creates* the AUR package — no separate registration step.

Users then install with any AUR helper: `yay -S agent-console-bin`.

## Owner checklist (Carlos, one time)

1. **AUR account**: create it at <https://aur.archlinux.org/register> (plain
   email registration, no approval queue).
2. **SSH key**: the CI needs a keypair whose public half is on your AUR
   account. Recommended: a dedicated key just for AUR (not your personal
   key). Either generate it yourself:

   ```bash
   ssh-keygen -t ed25519 -N "" -C "aur@agent-console" -f ~/.ssh/aur_agent_console
   ```

   …then paste the contents of `~/.ssh/aur_agent_console.pub` into
   <https://aur.archlinux.org/account/> → *SSH Public Key*, and save the
   PRIVATE file's contents as the repo secret **`AUR_SSH_PRIVATE_KEY`**
   (agent-console → Settings → Secrets → Actions).

   Or ask the agent to generate the pair and set the secret — you'd only
   paste the public key into the AUR page.
3. Done. The next release tag publishes `agent-console-bin` automatically;
   without the secret the job skips cleanly.

## Notes

- Package name is `agent-console-bin` per AUR convention for repackaged
  binaries (a source build would be `agent-console`). It `provides`/
  `conflicts` `agent-console`.
- Runtime deps mirror the .deb's: webkit2gtk-4.1, gtk3,
  libayatana-appindicator, alsa-lib, nodejs (PreToolUse hook), git.
- The PKGBUILD in-repo carries the last released version/sha as an honest
  snapshot; CI rewrites both on each publish.
