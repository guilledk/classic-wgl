# Tear-free picom on NixOS + i3

Add to `/etc/nixos/configuration.nix`:

```nix
services.picom = {
  enable = true;
  backend = "egl";        # most reliable on AMD (amdgpu/Mesa); use "glx" if egl misbehaves
  vSync = true;           # NOTE capital S — module default is FALSE, which causes tearing
  settings = {
    unredir-if-possible = true;
  };
};
```

Apply:

```bash
sudo nixos-rebuild switch
systemctl --user daemon-reload
systemctl --user restart picom
```

That's it — the module generates a systemd user unit (`picom.service`) that
autostarts with your graphical session, so **no i3 config / `exec picom` line is
needed**.

Verify:

```bash
pgrep -ax picom   # should show: picom --config /nix/store/...-picom.conf
```

## Gotchas learned the hard way

- The module's default `vsync = false` is the #1 reason people still see
  tearing — always set `vSync = true` (capital S).
- These picom builds have **no default backend**; bare `picom` fails with
  "Backend not specified". The service passes `--config`, so only the module's
  invocation works.
- `unredir-if-possible = true` lets fullscreen windows skip picom (direct
  scanout), so adaptive sync/VRR still works and the app's own vsync handles
  fullscreen. If you see fullscreen-only tearing, flip it to `false`.
- If a browser/game still tears, check the WebGL context —
  `desynchronized: true` bypasses the compositor and needs removing.
