//! The single source of truth for the host-import surface (the "console SDK").
//!
//! Every backend exposes the same 69 `env` host imports.  The *bodies* live in
//! the shared [`GuestHost`] (`sdk.rs`); this macro generates only the thin
//! linker/closure layer that marshals arguments in and out of guest linear
//! memory and forwards to `GuestHost`.  The wasmi and wasmtime backends differ
//! solely in their `Caller`/`Memory` types, so they both expand this one macro
//! (passing their own `read_str`/`write_*` helpers).
//!
//! When adding an import, edit *this* macro, the web/worker backends, and
//! `tests/guest.rs` (see `classic-guest` skill §7).

/// Generate the body of `install_imports` for the wasmi and wasmtime backends.
///
/// `$linker` is the `&mut Linker<Host>` to register into, `$host` the store
/// host type (wasmi's `WasmiHost` or wasmtime's `WasmtimeHost`), and the
/// remaining arguments are the backend-local memory-marshalling helpers
/// (`read_str`, `write_str`, `write_bytes`, `write_f64_pair`, `write_f64_triple`).
macro_rules! install_host_imports {
    ($linker:ident, $host:ty, $read_str:path, $read_bytes:path, $write_str:path, $write_bytes:path, $write_f64_pair:path, $write_f64_triple:path) => {{
        let m = crate::abi::HOST_MODULE;

        $linker.func_wrap(m, "log", |mut caller: Caller<'_, $host>, ptr: i32, len: i32| {
            let msg = $read_str(&mut caller, ptr, len);
            caller.data_mut().guest_mut().log(&msg);
        })?;

        $linker.func_wrap(
            m,
            "spawn",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().guest_mut().spawn(&name)
            },
        )?;

        $linker.func_wrap(
            m,
            "despawn",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().guest_mut().despawn(&name)
            },
        )?;

        $linker.func_wrap(
            m,
            "has",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().guest_mut().has(&name)
            },
        )?;

        $linker.func_wrap(
            m,
            "names",
            |mut caller: Caller<'_, $host>, out_ptr: i32, out_cap: i32| -> i32 {
                let json = caller.data_mut().guest_mut().names();
                if out_cap < json.len() as i32 {
                    return -1;
                }
                $write_str(&mut caller, out_ptr, &json)
            },
        )?;

        $linker.func_wrap(
            m,
            "set_pos",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32, x: f64, y: f64, z: f64| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().guest_mut().set_pos(&name, x, y, z)
            },
        )?;

        $linker.func_wrap(
            m,
            "get_pos",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32, out_ptr: i32| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                let Some((x, y, z)) = caller.data_mut().guest_mut().get_pos(&name) else {
                    return 0;
                };
                $write_f64_triple(&mut caller, out_ptr, x, y, z);
                1
            },
        )?;

        $linker.func_wrap(m, "mouse", |mut caller: Caller<'_, $host>, out_ptr: i32| -> i32 {
            let (x, y) = caller.data_mut().guest_mut().mouse();
            $write_f64_pair(&mut caller, out_ptr, x, y);
            1
        })?;

        $linker.func_wrap(
            m,
            "mouse_iso",
            |mut caller: Caller<'_, $host>, out_ptr: i32| -> i32 {
                let Some((x, y)) = caller.data_mut().guest_mut().mouse_iso() else {
                    return 0;
                };
                $write_f64_pair(&mut caller, out_ptr, x, y);
                1
            },
        )?;

        $linker.func_wrap(
            m,
            "iso_to_screen",
            |mut caller: Caller<'_, $host>, x: f64, y: f64, out_ptr: i32| -> i32 {
                let Some((sx, sy)) = caller.data_mut().guest_mut().iso_to_screen(x, y) else {
                    return 0;
                };
                $write_f64_pair(&mut caller, out_ptr, sx, sy);
                1
            },
        )?;

        $linker.func_wrap(
            m,
            "height_at",
            |mut caller: Caller<'_, $host>, x: f64, y: f64| -> f64 {
                caller.data_mut().guest_mut().height_at(x, y)
            },
        )?;

        $linker.func_wrap(
            m,
            "set_anim",
            |mut caller: Caller<'_, $host>,
             ptr: i32,
             len: i32,
             anim_ptr: i32,
             anim_len: i32|
             -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                let anim = $read_str(&mut caller, anim_ptr, anim_len);
                caller.data_mut().guest_mut().set_anim(&name, &anim)
            },
        )?;

        $linker.func_wrap(
            m,
            "start_anim",
            |mut caller: Caller<'_, $host>,
             ptr: i32,
             len: i32,
             anim_ptr: i32,
             anim_len: i32,
             repeat: i32|
             -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                let anim = $read_str(&mut caller, anim_ptr, anim_len);
                caller.data_mut().guest_mut().start_anim(&name, &anim, repeat)
            },
        )?;

        $linker.func_wrap(m, "agent_selected", |mut caller: Caller<'_, $host>| -> i32 {
            caller.data_mut().guest_mut().agent_selected()
        })?;

        $linker.func_wrap(m, "ui_consumed_click", |mut caller: Caller<'_, $host>| -> i32 {
            caller.data_mut().guest_mut().ui_consumed_click()
        })?;

        $linker.func_wrap(m, "delta", |mut caller: Caller<'_, $host>| -> f64 {
            caller.data_mut().guest_mut().delta()
        })?;

        $linker.func_wrap(m, "elapsed", |mut caller: Caller<'_, $host>| -> f64 {
            caller.data_mut().guest_mut().elapsed()
        })?;

        $linker.func_wrap(m, "was_pressed", |mut caller: Caller<'_, $host>, btn: i32| -> i32 {
            caller.data_mut().guest_mut().was_pressed(btn)
        })?;

        $linker.func_wrap(
            m,
            "key_down",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let key = $read_str(&mut caller, ptr, len);
                caller.data_mut().guest_mut().key_down(&key)
            },
        )?;

        $linker.func_wrap(
            m,
            "was_key_pressed",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let key = $read_str(&mut caller, ptr, len);
                caller.data_mut().guest_mut().was_key_pressed(&key)
            },
        )?;

        $linker.func_wrap(
            m,
            "set_tile",
            |mut caller: Caller<'_, $host>, x: i32, y: i32, id: i32| -> i32 {
                caller.data_mut().guest_mut().set_tile(x, y, id)
            },
        )?;

        $linker.func_wrap(
            m,
            "set_height",
            |mut caller: Caller<'_, $host>, x: i32, y: i32, h: f64| -> i32 {
                caller.data_mut().guest_mut().set_height(x, y, h)
            },
        )?;

        $linker.func_wrap(m, "rebuild_terrain", |mut caller: Caller<'_, $host>| -> i32 {
            caller.data_mut().guest_mut().rebuild_terrain()
        })?;

        $linker.func_wrap(
            m,
            "find_path",
            |mut caller: Caller<'_, $host>,
             sx: i32,
             sy: i32,
             ex: i32,
             ey: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let cells = caller.data_mut().guest_mut().find_path(sx, sy, ex, ey);
                let mut bytes = Vec::with_capacity(cells.len() * 8);
                for (x, y) in &cells {
                    bytes.extend_from_slice(&x.to_le_bytes());
                    bytes.extend_from_slice(&y.to_le_bytes());
                }
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                cells.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "get_camera",
            |mut caller: Caller<'_, $host>, out_ptr: i32| -> i32 {
                let (x, y, s) = caller.data_mut().guest_mut().get_camera();
                $write_f64_triple(&mut caller, out_ptr, x, y, s);
                1
            },
        )?;

        $linker.func_wrap(
            m,
            "set_camera",
            |mut caller: Caller<'_, $host>, x: f64, y: f64, scale: f64| -> i32 {
                caller.data_mut().guest_mut().set_camera(x, y, scale)
            },
        )?;

        $linker.func_wrap(m, "set_grid", |mut caller: Caller<'_, $host>, show: i32| -> i32 {
            caller.data_mut().guest_mut().set_grid(show)
        })?;

        $linker.func_wrap(
            m,
            "pick_at",
            |mut caller: Caller<'_, $host>, x: f64, y: f64, out_ptr: i32, out_cap: i32| -> i32 {
                let name = caller.data_mut().guest_mut().pick_at(x, y);
                if out_cap < name.len() as i32 {
                    return -1;
                }
                $write_str(&mut caller, out_ptr, &name)
            },
        )?;

        $linker.func_wrap(m, "mouse_down", |mut caller: Caller<'_, $host>, btn: i32| -> i32 {
            caller.data_mut().guest_mut().mouse_down(btn)
        })?;

        $linker.func_wrap(
            m,
            "mouse_released",
            |mut caller: Caller<'_, $host>, btn: i32| -> i32 {
                caller.data_mut().guest_mut().mouse_released(btn)
            },
        )?;

        $linker.func_wrap(m, "mouse_wheel", |mut caller: Caller<'_, $host>| -> f64 {
            caller.data_mut().guest_mut().mouse_wheel()
        })?;

        $linker.func_wrap(
            m,
            "key_up",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let key = $read_str(&mut caller, ptr, len);
                caller.data_mut().guest_mut().key_up(&key)
            },
        )?;

        $linker.func_wrap(
            m,
            "get_light",
            |mut caller: Caller<'_, $host>, out_ptr: i32| -> i32 {
                let (a, d, c) = caller.data_mut().guest_mut().get_light();
                let mut buf = Vec::with_capacity(72);
                for v in a.iter().chain(d.iter()).chain(c.iter()) {
                    buf.extend_from_slice(&v.to_le_bytes());
                }
                $write_bytes(&mut caller, out_ptr, &buf);
                1
            },
        )?;

        $linker.func_wrap(
            m,
            "set_light",
            |mut caller: Caller<'_, $host>,
             a0: f64,
             a1: f64,
             a2: f64,
             d0: f64,
             d1: f64,
             d2: f64,
             c0: f64,
             c1: f64,
             c2: f64|
             -> i32 {
                caller.data_mut().guest_mut().set_light(a0, a1, a2, d0, d1, d2, c0, c1, c2)
            },
        )?;

        $linker.func_wrap(
            m,
            "spawn_rect",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             x: f64,
             y: f64,
             w: f64,
             h: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                caller.data_mut().guest_mut().spawn_rect(&name, x, y, w, h, r, g, b, a)
            },
        )?;

        $linker.func_wrap(
            m,
            "spawn_text",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             x: f64,
             y: f64,
             text_ptr: i32,
             text_len: i32,
             scale: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                let text = $read_str(&mut caller, text_ptr, text_len);
                caller.data_mut().guest_mut().spawn_text(&name, x, y, &text, scale, r, g, b, a)
            },
        )?;

        $linker.func_wrap(
            m,
            "set_text",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             text_ptr: i32,
             text_len: i32|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                let text = $read_str(&mut caller, text_ptr, text_len);
                caller.data_mut().guest_mut().set_text(&name, &text)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_container",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             w: f64,
             h: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                caller.data_mut().guest_mut().ui_container(&name, w, h, r, g, b, a)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_text",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             text_ptr: i32,
             text_len: i32,
             scale: f64,
             max_width: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64,
             justify: i32|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                let text = $read_str(&mut caller, text_ptr, text_len);
                caller
                    .data_mut()
                    .guest_mut()
                    .ui_text(&name, &text, scale, max_width, r, g, b, a, justify)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_button",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             text_ptr: i32,
             text_len: i32,
             w: f64,
             h: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                let text = $read_str(&mut caller, text_ptr, text_len);
                caller.data_mut().guest_mut().ui_button(&name, &text, w, h, r, g, b, a)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_array",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             vertical: i32,
             align: i32,
             spacing: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                caller.data_mut().guest_mut().ui_array(&name, vertical, align, spacing, r, g, b, a)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_padding",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             top: f64,
             right: f64,
             bottom: f64,
             left: f64,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                caller
                    .data_mut()
                    .guest_mut()
                    .ui_padding(&name, top, right, bottom, left, r, g, b, a)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_sprite",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             texture_ptr: i32,
             texture_len: i32,
             w: f64,
             h: f64,
             frame: f64,
             tsx: f64,
             tsy: f64|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                let texture = $read_str(&mut caller, texture_ptr, texture_len);
                caller.data_mut().guest_mut().ui_sprite(&name, &texture, w, h, frame, tsx, tsy)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_add_child",
            |mut caller: Caller<'_, $host>,
             parent_ptr: i32,
             parent_len: i32,
             child_ptr: i32,
             child_len: i32,
             self_anchor: i32,
             child_anchor: i32|
             -> i32 {
                let parent = $read_str(&mut caller, parent_ptr, parent_len);
                let child = $read_str(&mut caller, child_ptr, child_len);
                caller.data_mut().guest_mut().ui_add_child(
                    &parent,
                    &child,
                    self_anchor,
                    child_anchor,
                )
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_add_to_root",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             self_anchor: i32,
             child_anchor: i32|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                caller.data_mut().guest_mut().ui_add_to_root(&name, self_anchor, child_anchor)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_set_size",
            |mut caller: Caller<'_, $host>, name_ptr: i32, name_len: i32, w: f64, h: f64| -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                caller.data_mut().guest_mut().ui_set_size(&name, w, h)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_set_anchor",
            |mut caller: Caller<'_, $host>, name_ptr: i32, name_len: i32, anchor: i32| -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                caller.data_mut().guest_mut().ui_set_anchor(&name, anchor)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_set_color",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             r: f64,
             g: f64,
             b: f64,
             a: f64|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                caller.data_mut().guest_mut().ui_set_color(&name, r, g, b, a)
            },
        )?;

        $linker.func_wrap(
            m,
            "ui_set_fixed",
            |mut caller: Caller<'_, $host>, name_ptr: i32, name_len: i32, fixed: i32| -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                caller.data_mut().guest_mut().ui_set_fixed(&name, fixed)
            },
        )?;

        $linker.func_wrap(
            m,
            "subscribe",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().guest_mut().subscribe(&name)
            },
        )?;

        $linker.func_wrap(
            m,
            "poll_event",
            |mut caller: Caller<'_, $host>, out_ptr: i32, out_cap: i32| -> i32 {
                let Some((kind, name)) = caller.data_mut().guest_mut().poll_event() else {
                    return 0;
                };
                let mut bytes = Vec::with_capacity(8 + name.len());
                bytes.extend_from_slice(&kind.to_le_bytes());
                bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
                bytes.extend_from_slice(name.as_bytes());
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                1
            },
        )?;

        $linker.func_wrap(
            m,
            "spawn_collider",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             x: f64,
             y: f64,
             w: f64,
             h: f64|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                caller.data_mut().guest_mut().spawn_collider(&name, x, y, w, h)
            },
        )?;

        $linker.func_wrap(
            m,
            "get_anim",
            |mut caller: Caller<'_, $host>,
             name_ptr: i32,
             name_len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let name = $read_str(&mut caller, name_ptr, name_len);
                let Some((anim, frame)) = caller.data_mut().guest_mut().get_anim(&name) else {
                    return 0;
                };
                let mut bytes = Vec::with_capacity(12 + anim.len());
                bytes.extend_from_slice(&frame.to_le_bytes());
                bytes.extend_from_slice(&(anim.len() as u32).to_le_bytes());
                bytes.extend_from_slice(anim.as_bytes());
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                1
            },
        )?;

        $linker.func_wrap(
            m,
            "has_resource",
            |mut caller: Caller<'_, $host>, kind: i32, ptr: i32, len: i32| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                caller.data_mut().guest_mut().has_resource(kind, &name)
            },
        )?;

        $linker.func_wrap(
            m,
            "texture_size",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32, out_ptr: i32| -> i32 {
                let name = $read_str(&mut caller, ptr, len);
                let Some((w, h)) = caller.data_mut().guest_mut().texture_size(&name) else {
                    return 0;
                };
                $write_f64_pair(&mut caller, out_ptr, w, h);
                1
            },
        )?;

        // ---- Bulk noise fields (host generates → guest buffer) --------------

        $linker.func_wrap(
            m,
            "fbm_field",
            |mut caller: Caller<'_, $host>,
             w: i32,
             h: i32,
             seed_ptr: i32,
             seed_len: i32,
             octaves: i32,
             freq: f64,
             lacunarity: f64,
             gain: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                let field = caller.data_mut().guest_mut().fbm_field(
                    w,
                    h,
                    &seed,
                    octaves.max(0) as u32,
                    freq,
                    lacunarity,
                    gain,
                );
                let bytes = crate::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "ridged_field",
            |mut caller: Caller<'_, $host>,
             w: i32,
             h: i32,
             seed_ptr: i32,
             seed_len: i32,
             octaves: i32,
             freq: f64,
             lacunarity: f64,
             gain: f64,
             warp_amp: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                let field = caller.data_mut().guest_mut().ridged_field(
                    w,
                    h,
                    &seed,
                    octaves.max(0) as u32,
                    freq,
                    lacunarity,
                    gain,
                    warp_amp,
                );
                let bytes = crate::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "billow_field",
            |mut caller: Caller<'_, $host>,
             w: i32,
             h: i32,
             seed_ptr: i32,
             seed_len: i32,
             octaves: i32,
             freq: f64,
             lacunarity: f64,
             gain: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                let field = caller.data_mut().guest_mut().billow_field(
                    w,
                    h,
                    &seed,
                    octaves.max(0) as u32,
                    freq,
                    lacunarity,
                    gain,
                );
                let bytes = crate::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "tiling_field",
            |mut caller: Caller<'_, $host>,
             w: i32,
             h: i32,
             seed_ptr: i32,
             seed_len: i32,
             period: f64,
             octaves: i32,
             radius: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                let field = caller.data_mut().guest_mut().tiling_field(
                    w,
                    h,
                    &seed,
                    period,
                    octaves.max(0) as u32,
                    radius,
                );
                let bytes = crate::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "noise_field",
            |mut caller: Caller<'_, $host>,
             w: i32,
             h: i32,
             seed_ptr: i32,
             seed_len: i32,
             freq_x: f64,
             freq_y: f64,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                let field = caller.data_mut().guest_mut().noise_field(w, h, &seed, freq_x, freq_y);
                let bytes = crate::abi::f32_array_bytes(&field);
                if bytes.len() > out_cap.max(0) as usize {
                    return -1;
                }
                $write_bytes(&mut caller, out_ptr, &bytes);
                bytes.len() as i32
            },
        )?;

        $linker.func_wrap(
            m,
            "noise2d",
            |mut caller: Caller<'_, $host>, seed_ptr: i32, seed_len: i32, x: f64, y: f64| -> f64 {
                let seed = $read_str(&mut caller, seed_ptr, seed_len);
                caller.data_mut().guest_mut().noise2d(&seed, x, y)
            },
        )?;

        // ---- Bulk terrain upload (guest generates → host stores) ------------

        $linker.func_wrap(
            m,
            "set_tiles",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let tiles = crate::abi::bytes_to_u32(&$read_bytes(&mut caller, ptr, len));
                caller.data_mut().guest_mut().set_tiles(&tiles)
            },
        )?;

        $linker.func_wrap(
            m,
            "set_heights",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let heights = crate::abi::bytes_to_f32(&$read_bytes(&mut caller, ptr, len));
                caller.data_mut().guest_mut().set_heights(&heights)
            },
        )?;

        $linker.func_wrap(
            m,
            "set_nav",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32| -> i32 {
                let nav = crate::abi::bytes_to_u32(&$read_bytes(&mut caller, ptr, len));
                caller.data_mut().guest_mut().set_nav(&nav)
            },
        )?;

        $linker.func_wrap(
            m,
            "set_tileset",
            |mut caller: Caller<'_, $host>, ptr: i32, len: i32, w: i32, h: i32| -> i32 {
                let rgba = $read_bytes(&mut caller, ptr, len);
                caller.data_mut().guest_mut().set_tileset(&rgba, w.max(0) as u32, h.max(0) as u32)
            },
        )?;

        $linker.func_wrap(
            m,
            "commit_terrain",
            |mut caller: Caller<'_, $host>, height_scale: f64| -> i32 {
                caller.data_mut().guest_mut().commit_terrain(height_scale)
            },
        )?;

        Ok(())
    }};
}

pub(crate) use install_host_imports;
