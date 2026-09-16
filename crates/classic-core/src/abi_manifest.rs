//! The declarative host-import table: the single source of truth for the guest
//! ABI's `env` import surface (the "console SDK").
//!
//! [`for_each_host_import!`] expands a callback over every import.  Each entry
//! records the import `name`, its typed parameters, its return kind, and the set
//! of backends that expose it:
//!
//! ```text
//! name(param: ParamKind, ...) -> RetKind [backend ...];
//! ```
//!
//! **Parameter kinds** (how an argument crosses the boundary):
//!
//! | Kind | Wasm params | Host value |
//! |---|---|---|
//! | `i32` / `f64` | one `i32` / `f64` | passed through |
//! | `u32` | one `i32` | `v.max(0) as u32` |
//! | `str` | `ptr, len` | `&str` read from guest memory (lossy UTF-8) |
//! | `bytes` | `ptr, len` | `&[u8]` |
//! | `bytes_owned` | `ptr, len` | `Vec<u8>` (moved into the host) |
//! | `f32s` / `u32s` | `ptr, len` | `&[f32]` / `&[u32]` decoded little-endian |
//!
//! **Return kinds** (out-buffer kinds append trailing `out_ptr[, out_cap]`
//! params and return `i32`):
//!
//! | Kind | Trailing params | Host value → guest |
//! |---|---|---|
//! | `unit` / `i32` / `f64` | — | returned directly |
//! | `json` | `out_ptr, out_cap` | `String`; `-1` if `out_cap < len`, else bytes written |
//! | `pair` / `triple` | `out_ptr` | `(f64, f64[, f64])` written LE; returns `1` |
//! | `pair_opt` / `triple_opt` | `out_ptr` | `Option<..>`; `0` on `None`, else written + `1` |
//! | `light` | `out_ptr` | `([f64; 3], [f64; 3], [f64; 3])` as nine LE `f64`; returns `1` |
//! | `f32s` | `out_ptr, out_cap` | `Vec<f32>` as LE bytes; `-1` if too small, else bytes written |
//! | `bytes_out` | `out_ptr, out_cap` | `Vec<u8>`; `-1` if too small, else bytes written |
//! | `path_poll` | `out_ptr, out_cap` | `PathPoll`; `0` pending, `-1` no path, `-2` too small, else cell count |
//! | `path_opt` | `out_ptr, out_cap` | `Option<Vec<GridCell>>`; `-1` none, `-2` too small, else cell count |
//! | `task_poll` | `out_ptr, out_cap` | `Option<Result<Vec<u8>, String>>`; `0` pending, `-1` error (logged), `-2` too small, else bytes |
//! | `event` | `out_ptr, out_cap` | `Option<(u32, String)>` as `kind:u32 len:u32 name`; `0` none, `-1` too small, else `1` |
//! | `anim` | `out_ptr, out_cap` | `Option<(String, f64)>` as `frame:f64 len:u32 name`; `0` none, `-1` too small, else `1` |
//!
//! **Backends**: `native` (the wasmi + wasmtime foreground runtimes), `web`
//! (browser-native `WebAssembly`), `worker` (the untrusted SAB `Worker`),
//! `tier3` (the background worker guest's real surface) and `tier3_trap`
//! (registered in the background worker surface as a trap stub).
//!
//! Backend import layers are generated from this table by [`host_imports!`],
//! which holds the per-kind marshalling once and calls a small per-backend
//! frontend (the wasmi/wasmtime one is [`link_host_imports!`]).
//! [`HOST_IMPORTS`] is a runtime descriptor of the same table for tooling
//! (signature checks, backend-subset tests).

/// Expand `$cb! { ctx... entries... }` over every host import.
///
/// The callback path is passed as a bracketed token list so `$crate`-qualified
/// macros work: `for_each_host_import!([$crate::some_cb] { ctx tokens })`.
#[macro_export]
macro_rules! for_each_host_import {
    ([$($cb:tt)*] { $($ctx:tt)* }) => {
        $($cb)*! { $($ctx)*
            // ---- Entities, sprites, input --------------------------------
            log(msg: str) -> unit [native web worker tier3];
            spawn(name: str) -> i32 [native web worker tier3_trap];
            despawn(name: str) -> i32 [native web worker tier3_trap];
            has(name: str) -> i32 [native web worker tier3_trap];
            names() -> json [native web worker];
            set_pos(name: str, x: f64, y: f64, z: f64) -> i32 [native web worker tier3_trap];
            get_pos(name: str) -> triple_opt [native web worker tier3_trap];
            set_sprite_frame(name: str, frame: f64) -> i32 [native web worker];
            get_sprite_frame(name: str) -> f64 [native web worker];
            set_sprite_color(name: str, r: f64, g: f64, b: f64, a: f64) -> i32 [native web worker];
            set_sprite_offset(name: str, dx: f64, dy: f64, dz: f64) -> i32 [native web worker];
            spawn_sprite_clone(template: str, name: str) -> i32 [native web worker];
            mouse() -> pair [native web worker];
            mouse_iso() -> pair_opt [native web worker];
            iso_to_screen(x: f64, y: f64) -> pair_opt [native web worker];
            height_at(x: f64, y: f64) -> f64 [native web worker tier3_trap];
            set_anim(name: str, anim: str) -> i32 [native web worker tier3_trap];
            start_anim(name: str, anim: str, repeat: i32) -> i32 [native web worker tier3_trap];
            set_enabled(name: str, enabled: i32) -> i32 [native web worker];
            agent_selected() -> i32 [native web worker];
            ui_consumed_click() -> i32 [native web worker];
            delta() -> f64 [native web worker];
            elapsed() -> f64 [native web worker];
            was_pressed(btn: i32) -> i32 [native web worker];
            key_down(key: str) -> i32 [native web worker];
            was_key_pressed(key: str) -> i32 [native web worker];

            // ---- Terrain edits + pathfinding + background tasks ----------
            set_tile(x: i32, y: i32, id: i32) -> i32 [native web worker tier3_trap];
            set_height(x: i32, y: i32, h: f64) -> i32 [native web worker tier3_trap];
            rebuild_terrain() -> i32 [native web worker tier3_trap];
            request_path(sx: i32, sy: i32, ex: i32, ey: i32) -> i32 [native web worker tier3_trap];
            poll_path(id: i32) -> path_poll [native web worker tier3_trap];
            spawn_task(entry: str, arg: bytes) -> i32 [native web worker];
            poll_task(id: i32) -> task_poll [native web worker];

            // ---- Vehicles -------------------------------------------------
            vehicle_teleport(name: str, x: f64, y: f64) -> i32 [native web worker];
            vehicle_spawn(def: str, name: str, x: f64, y: f64) -> i32 [native web worker];
            vehicle_goto(name: str, tx: i32, ty: i32) -> i32 [native web worker];
            vehicle_goto_poll(id: i32) -> i32 [native web worker];
            vehicle_stop(name: str) -> i32 [native web worker];
            vehicle_set_speed(name: str, speed: f64) -> i32 [native web worker];
            vehicle_probe(name: str, tx: i32, ty: i32) -> i32 [native web worker];
            vehicle_probe_clear(name: str) -> i32 [native web worker];
            vehicle_footprint_radius(name: str) -> f64 [native web worker];

            // ---- Selection + inventory -----------------------------------
            selected_names() -> json [native web worker];
            selection_clear() -> i32 [native web worker];
            inventory_dump(name: str) -> json [native web worker];
            inventory_capacity(name: str) -> i32 [native web worker];
            inventory_add(name: str, item: str, n: i32) -> i32 [native web worker];
            inventory_remove(name: str, item: str, n: i32) -> i32 [native web worker];
            inventory_transfer(from: str, to: str, item: str, n: i32) -> i32 [native web worker];
            item_def(name: str) -> json [native web worker];
            inventory_ui_show(name: str) -> i32 [native web worker];

            // ---- Camera, picking, more input -----------------------------
            get_camera() -> triple [native web worker tier3_trap];
            set_camera(x: f64, y: f64, scale: f64) -> i32 [native web worker tier3_trap];
            set_grid(show: i32) -> i32 [native web worker tier3_trap];
            pick_at(x: f64, y: f64, filter: str) -> json [native web worker];
            mouse_down(btn: i32) -> i32 [native web worker];
            set_collider_blocks_nav(name: str, blocks: i32) -> i32 [native web worker];
            mouse_released(btn: i32) -> i32 [native web worker];
            mouse_wheel() -> f64 [native web worker];
            key_up(key: str) -> i32 [native web worker];

            // ---- Lights ---------------------------------------------------
            get_light() -> light [native web worker tier3_trap];
            set_light(
                a0: f64, a1: f64, a2: f64, d0: f64, d1: f64, d2: f64, c0: f64, c1: f64, c2: f64
            ) -> i32 [native web worker tier3_trap];
            light_spawn(
                kind: i32, x: f64, y: f64, z: f64, r: f64, g: f64, b: f64, intensity: f64,
                radius: f64, ttl: f64
            ) -> i32 [native web worker];
            light_set(
                handle: i32, x: f64, y: f64, z: f64, r: f64, g: f64, b: f64, intensity: f64,
                radius: f64
            ) -> i32 [native web worker];
            light_release(handle: i32) -> i32 [native web worker];

            // ---- Screen-space + managed UI -------------------------------
            spawn_rect(
                name: str, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64
            ) -> i32 [native web worker];
            spawn_text(
                name: str, x: f64, y: f64, text: str, scale: f64, r: f64, g: f64, b: f64, a: f64
            ) -> i32 [native web worker];
            set_text(name: str, text: str) -> i32 [native web worker];
            ui_container(name: str, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) -> i32
                [native web worker];
            ui_text(
                name: str, text: str, scale: f64, max_width: f64, r: f64, g: f64, b: f64, a: f64,
                justify: i32
            ) -> i32 [native web worker];
            ui_button(name: str, text: str, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) -> i32
                [native web worker];
            ui_array(
                name: str, vertical: i32, align: i32, spacing: f64, r: f64, g: f64, b: f64, a: f64
            ) -> i32 [native web worker];
            ui_padding(
                name: str, top: f64, right: f64, bottom: f64, left: f64, r: f64, g: f64, b: f64,
                a: f64
            ) -> i32 [native web worker];
            ui_sprite(
                name: str, texture: str, w: f64, h: f64, frame: f64, tsx: f64, tsy: f64
            ) -> i32 [native web worker];
            ui_add_child(parent: str, child: str, self_anchor: i32, child_anchor: i32) -> i32
                [native web worker];
            ui_add_to_root(name: str, self_anchor: i32, child_anchor: i32) -> i32
                [native web worker];
            ui_set_size(name: str, w: f64, h: f64) -> i32 [native web worker];
            ui_set_anchor(name: str, anchor: i32) -> i32 [native web worker];
            ui_set_color(name: str, r: f64, g: f64, b: f64, a: f64) -> i32 [native web worker];
            ui_set_fixed(name: str, fixed: i32) -> i32 [native web worker];
            subscribe(name: str) -> i32 [native web worker];
            poll_event() -> event [native web worker];
            spawn_collider(name: str, x: f64, y: f64, w: f64, h: f64) -> i32 [native web worker];
            get_anim(name: str) -> anim [native web worker];
            has_resource(kind: i32, name: str) -> i32 [native web worker];
            texture_size(name: str) -> pair_opt [native web worker];

            // ---- Bulk noise fields (host generates -> guest buffer) -------
            fbm_field(
                w: i32, h: i32, seed: str, octaves: u32, freq: f64, lacunarity: f64, gain: f64
            ) -> f32s [native web worker tier3];
            ridged_field(
                w: i32, h: i32, seed: str, octaves: u32, freq: f64, lacunarity: f64, gain: f64,
                warp_amp: f64
            ) -> f32s [native web worker tier3];
            billow_field(
                w: i32, h: i32, seed: str, octaves: u32, freq: f64, lacunarity: f64, gain: f64
            ) -> f32s [native web worker tier3];
            tiling_field(w: i32, h: i32, seed: str, period: f64, octaves: u32, radius: f64) -> f32s
                [native web worker tier3];
            noise_field(w: i32, h: i32, seed: str, freq_x: f64, freq_y: f64) -> f32s
                [native web worker tier3];
            noise2d(seed: str, x: f64, y: f64) -> f64 [native web worker tier3];

            // ---- Bulk terrain upload (guest generates -> host stores) -----
            set_tiles(tiles: u32s) -> i32 [native web worker tier3_trap];
            set_heights(heights: f32s) -> i32 [native web worker tier3_trap];
            set_nav(nav: u32s) -> i32 [native web worker tier3_trap];
            set_tileset(rgba: bytes, w: u32, h: u32) -> i32 [native web worker tier3_trap];
            commit_terrain(height_scale: f64) -> i32 [native web worker tier3_trap];

            // ---- Field-buffer registry + grid kernels ---------------------
            alloc_field(name: str, w: i32, h: i32, dtype: i32) -> i32 [native web worker tier3];
            free_field(name: str) -> i32 [native web worker tier3];
            write_field(name: str, data: f32s) -> i32 [native web worker tier3];
            write_field_u32(name: str, data: u32s) -> i32 [native web worker tier3];
            read_field(name: str) -> f32s [native web worker tier3];
            map_field(op: i32, dst: str, src: str) -> i32 [native web worker tier3];
            map_scalar(op: i32, dst: str, scalar: f64) -> i32 [native web worker tier3];
            blur_box_field(name: str, radius: i32) -> i32 [native web worker tier3];
            relax_slopes_field(
                name: str, max_slope: f64, iterations: i32, tolerance: f64, pinned: str
            ) -> f64 [native web worker tier3];
            gradient_magnitude_field(heights: str, dst: str) -> i32 [native web worker tier3];
            threshold_le_field(src: str, dst: str, t: f64) -> i32 [native web worker tier3];
            prune_components_field(name: str) -> i32 [native web worker tier3];
            reduce_field(name: str, op: i32) -> f64 [native web worker tier3];

            // ---- Background worker guest only (Tier 3) --------------------
            find_path(sx: i32, sy: i32, ex: i32, ey: i32) -> path_opt [tier3];
            task_arg() -> bytes_out [tier3];
            task_return(bytes: bytes_owned) -> unit [tier3];
        }
    };
}

/// Generate one backend's host-import registrations from
/// [`for_each_host_import!`], through a backend *frontend* macro.
///
/// `backend` selects the entries (`native`, `web`, `worker` or `tier3`; the
/// `tier3` backend also registers a trap stub for every `tier3_trap` entry).
/// The per-kind marshalling logic lives here once; the frontend only supplies
/// how to reach guest memory and the host, and how to register an import.  It
/// is invoked as `frontend!(@hook extra ...)`, where `extra` is the opaque
/// token tree passed through from the call site:
///
/// | Hook | Form | Yields |
/// |---|---|---|
/// | `@host` | `extra caller` | the value implementing the import methods |
/// | `@read_str` / `@read_bytes` | `extra caller ptr len` | `String` / `Vec<u8>` |
/// | `@write_str` / `@write_bytes` | `extra caller out_ptr, value` | `i32` bytes written |
/// | `@write_pair` / `@write_triple` | `extra caller out_ptr, a, b[, c]` | (ignored) |
/// | `@register` | `extra caller name (ret) [raw: ty, ..] body` | a registration statement |
/// | `@register_trap` | `extra name (ret) [ty ..]` | a trap-stub registration (tier3) |
///
/// `caller` is an identifier the frontend may bind as a closure parameter (the
/// wasmi/wasmtime `Caller`); frontends without one ignore it.  Hooks that a
/// backend never needs may be left undefined.
#[macro_export]
macro_rules! host_imports {
    ($backend:ident, [$($frontend:tt)*], $extra:tt) => {
        $crate::for_each_host_import!([$crate::__host_import] {
            @table $backend [[$($frontend)*] $extra]
        });
    };
}

/// Register a backend's host imports into a wasmi/wasmtime-shaped `Linker`
/// (`func_wrap(module, name, |caller: Caller<'_, Host>, ..| ..)`).
///
/// Expands to a sequence of `$linker.func_wrap(..)?;` statements, so it must be
/// used inside a function returning a `Result` the linker error converts into.
/// `Caller` must be in scope at the call site.  `access` is the method chain
/// from the store data (`caller.data_mut()`) to the value implementing the
/// import methods (e.g. `[.guest_mut()]`, or `[]` when the store data is the
/// host itself).
///
/// - `native`: every `native` import, forwarded to same-named host methods.
/// - `tier3`: every `tier3` import, plus a trap stub (`Err($trap("name"))`) with
///   the real wasm signature for every `tier3_trap` import.
#[macro_export]
macro_rules! link_host_imports {
    (native {
        linker: $linker:ident,
        host: $host:ty,
        module: $module:expr,
        access: [$($acc:tt)*],
        read_str: $rs:path,
        read_bytes: $rb:path,
        write_str: $ws:path,
        write_bytes: $wb:path,
        write_f64_pair: $w2:path,
        write_f64_triple: $w3:path,
    }) => {
        $crate::host_imports!(native, [$crate::__linker_frontend], [
            native $linker, $host, $module, [$($acc)*], $rs, $rb, $ws, $wb, $w2, $w3,
        ]);
    };
    (tier3 {
        linker: $linker:ident,
        host: $host:ty,
        module: $module:expr,
        access: [$($acc:tt)*],
        read_str: $rs:path,
        read_bytes: $rb:path,
        write_bytes: $wb:path,
        err: $err:ty,
        trap: $trap:path,
    }) => {
        $crate::host_imports!(tier3, [$crate::__linker_frontend], [
            tier3 $linker, $host, $module, [$($acc)*], $rs, $rb, $wb, $err, $trap,
        ]);
    };
}

/// The wasmi/wasmtime `Linker` frontend behind [`link_host_imports!`].
#[doc(hidden)]
#[macro_export]
macro_rules! __linker_frontend {
    (@host [$tag:ident $linker:ident, $host:ty, $module:expr, [$($acc:tt)*], $($rest:tt)*]
        $c:ident) => {
        $c.data_mut() $($acc)*
    };
    (@read_str [$tag:ident $linker:ident, $host:ty, $module:expr, $acc:tt, $rs:path, $rb:path,
        $($rest:tt)*] $c:ident $p:ident $l:ident) => {
        $rs(&mut $c, $p, $l)
    };
    (@read_bytes [$tag:ident $linker:ident, $host:ty, $module:expr, $acc:tt, $rs:path, $rb:path,
        $($rest:tt)*] $c:ident $p:ident $l:ident) => {
        $rb(&mut $c, $p, $l)
    };
    (@write_str [native $linker:ident, $host:ty, $module:expr, $acc:tt, $rs:path, $rb:path,
        $ws:path, $wb:path, $w2:path, $w3:path,] $c:ident $ptr:expr, $v:expr) => {
        $ws(&mut $c, $ptr, $v)
    };
    (@write_bytes [native $linker:ident, $host:ty, $module:expr, $acc:tt, $rs:path, $rb:path,
        $ws:path, $wb:path, $w2:path, $w3:path,] $c:ident $ptr:expr, $v:expr) => {
        $wb(&mut $c, $ptr, $v)
    };
    (@write_bytes [tier3 $linker:ident, $host:ty, $module:expr, $acc:tt, $rs:path, $rb:path,
        $wb:path, $err:ty, $trap:path,] $c:ident $ptr:expr, $v:expr) => {
        $wb(&mut $c, $ptr, $v)
    };
    (@write_pair [native $linker:ident, $host:ty, $module:expr, $acc:tt, $rs:path, $rb:path,
        $ws:path, $wb:path, $w2:path, $w3:path,] $c:ident $ptr:expr, $a:expr, $b:expr) => {
        $w2(&mut $c, $ptr, $a, $b)
    };
    (@write_triple [native $linker:ident, $host:ty, $module:expr, $acc:tt, $rs:path, $rb:path,
        $ws:path, $wb:path, $w2:path, $w3:path,] $c:ident $ptr:expr, $a:expr, $b:expr,
        $z:expr) => {
        $w3(&mut $c, $ptr, $a, $b, $z)
    };
    (@register [$tag:ident $linker:ident, $host:ty, $module:expr, $($rest:tt)*] $c:ident
        $name:ident ($ret:ty) [$($p:ident: $t:ident,)*] {$($body:tt)*}) => {
        $linker.func_wrap(
            $module,
            stringify!($name),
            |mut $c: Caller<'_, $host>, $($p: $t,)*| -> $ret { $($body)* },
        )?;
    };
    (@register_trap [tier3 $linker:ident, $host:ty, $module:expr, $acc:tt, $rs:path, $rb:path,
        $wb:path, $err:ty, $trap:path,] $name:ident ($ret:ty) [$($t:ident)*]) => {
        $linker.func_wrap(
            $module,
            stringify!($name),
            |_caller: Caller<'_, $host>, $(_: $t,)*| -> Result<$ret, $err> {
                Err($trap(stringify!($name)))
            },
        )?;
    };
}

/// Internal muncher behind [`host_imports!`].
///
/// Context (`$ctx`): `[[frontend path tokens] extra]`.
#[doc(hidden)]
#[macro_export]
macro_rules! __host_import {
    // ---- frontend trampoline (`$fe` is the bracketed frontend path) --------
    (@fe [$($fe:tt)*] $($args:tt)*) => {
        $($fe)*!($($args)*)
    };

    // ---- table walk + backend selection ----------------------------------
    (@table $want:ident $ctx:tt
        $( $name:ident ( $($pn:ident : $pk:ident),* $(,)? ) -> $ret:ident [ $($be:ident)* ] ; )*
    ) => {
        $( $crate::__host_import!(
            @select $ctx $want [$($be)*] { $name ($($pn : $pk,)*) -> $ret }
        ); )*
    };

    (@select $ctx:tt $want:ident [] $entry:tt) => {};
    (@select $ctx:tt native [native $($rest:ident)*] $entry:tt) => {
        $crate::__host_import!(@real $ctx $entry);
    };
    (@select $ctx:tt web [web $($rest:ident)*] $entry:tt) => {
        $crate::__host_import!(@real $ctx $entry);
    };
    (@select $ctx:tt worker [worker $($rest:ident)*] $entry:tt) => {
        $crate::__host_import!(@real $ctx $entry);
    };
    (@select $ctx:tt tier3 [tier3 $($rest:ident)*] $entry:tt) => {
        $crate::__host_import!(@real $ctx $entry);
    };
    (@select $ctx:tt tier3 [tier3_trap $($rest:ident)*] $entry:tt) => {
        $crate::__host_import!(@trap $ctx $entry);
    };
    (@select $ctx:tt $want:ident [$other:ident $($rest:ident)*] $entry:tt) => {
        $crate::__host_import!(@select $ctx $want [$($rest)*] $entry);
    };

    // ---- real imports: munch params into [raw params] [reads] [call args] --
    (@real $ctx:tt { $name:ident ($($params:tt)*) -> $ret:ident }) => {
        $crate::__host_import!(@param $ctx $name $ret caller [] [] [] $($params)*);
    };

    (@param $ctx:tt $name:ident $ret:ident $c:ident [$($raw:tt)*] [$($rd:tt)*] [$($arg:tt)*]
        $p:ident : i32, $($rest:tt)*) => {
        $crate::__host_import!(@param $ctx $name $ret $c
            [$($raw)* $p: i32,] [$($rd)*] [$($arg)* $p,] $($rest)*);
    };
    (@param $ctx:tt $name:ident $ret:ident $c:ident [$($raw:tt)*] [$($rd:tt)*] [$($arg:tt)*]
        $p:ident : f64, $($rest:tt)*) => {
        $crate::__host_import!(@param $ctx $name $ret $c
            [$($raw)* $p: f64,] [$($rd)*] [$($arg)* $p,] $($rest)*);
    };
    (@param $ctx:tt $name:ident $ret:ident $c:ident [$($raw:tt)*] [$($rd:tt)*] [$($arg:tt)*]
        $p:ident : u32, $($rest:tt)*) => {
        $crate::__host_import!(@param $ctx $name $ret $c
            [$($raw)* $p: i32,] [$($rd)*] [$($arg)* $p.max(0) as u32,] $($rest)*);
    };
    (@param $ctx:tt $name:ident $ret:ident $c:ident [$($raw:tt)*] [$($rd:tt)*] [$($arg:tt)*]
        $p:ident : bytes_owned, $($rest:tt)*) => {
        $crate::__host_import!(@param $ctx $name $ret $c
            [$($raw)* ptr: i32, len: i32,] [$($rd)* {bytes ptr len value}] [$($arg)* value,]
            $($rest)*);
    };
    // `str` / `bytes` / `f32s` / `u32s`: read into a local, pass by reference.
    (@param $ctx:tt $name:ident $ret:ident $c:ident [$($raw:tt)*] [$($rd:tt)*] [$($arg:tt)*]
        $p:ident : $kind:ident, $($rest:tt)*) => {
        $crate::__host_import!(@param $ctx $name $ret $c
            [$($raw)* ptr: i32, len: i32,] [$($rd)* {$kind ptr len value}] [$($arg)* &value,]
            $($rest)*);
    };
    (@param $ctx:tt $name:ident $ret:ident $c:ident $raw:tt $rd:tt $arg:tt) => {
        $crate::__host_import!(@emit $ctx $name $ret $c $raw $rd $arg);
    };

    // ---- guest-memory reads ------------------------------------------------
    (@read [$fe:tt $x:tt] $c:ident {str $p:ident $l:ident $v:ident}) => {
        let $v = $crate::__host_import!(@fe $fe @read_str $x $c $p $l);
    };
    (@read [$fe:tt $x:tt] $c:ident {bytes $p:ident $l:ident $v:ident}) => {
        let $v = $crate::__host_import!(@fe $fe @read_bytes $x $c $p $l);
    };
    (@read [$fe:tt $x:tt] $c:ident {f32s $p:ident $l:ident $v:ident}) => {
        let $v = $crate::abi::bytes_to_f32(&$crate::__host_import!(@fe $fe @read_bytes $x $c $p $l));
    };
    (@read [$fe:tt $x:tt] $c:ident {u32s $p:ident $l:ident $v:ident}) => {
        let $v = $crate::abi::bytes_to_u32(&$crate::__host_import!(@fe $fe @read_bytes $x $c $p $l));
    };

    // ---- emit one real import, per return kind -----------------------------
    (@emit [$fe:tt $x:tt] $name:ident unit $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (()) [$($raw)*] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*);
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident i32 $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)*] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*)
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident f64 $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (f64) [$($raw)*] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*)
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident json $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32, out_cap: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let json = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*);
            if out_cap < json.len() as i32 {
                return -1;
            }
            $crate::__host_import!(@fe $fe @write_str $x $c out_ptr, &json)
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident pair $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let (x, y) = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*);
            $crate::__host_import!(@fe $fe @write_pair $x $c out_ptr, x, y);
            1
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident pair_opt $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let Some((x, y)) = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*) else {
                return 0;
            };
            $crate::__host_import!(@fe $fe @write_pair $x $c out_ptr, x, y);
            1
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident triple $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let (x, y, z) = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*);
            $crate::__host_import!(@fe $fe @write_triple $x $c out_ptr, x, y, z);
            1
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident triple_opt $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let Some((x, y, z)) = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*) else {
                return 0;
            };
            $crate::__host_import!(@fe $fe @write_triple $x $c out_ptr, x, y, z);
            1
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident light $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let (a, d, col) = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*);
            let mut buf = Vec::with_capacity(72);
            for v in a.iter().chain(d.iter()).chain(col.iter()) {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            $crate::__host_import!(@fe $fe @write_bytes $x $c out_ptr, &buf);
            1
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident f32s $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32, out_cap: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let field = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*);
            let bytes = $crate::abi::f32_array_bytes(&field);
            if bytes.len() > out_cap.max(0) as usize {
                return -1;
            }
            $crate::__host_import!(@fe $fe @write_bytes $x $c out_ptr, &bytes);
            bytes.len() as i32
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident bytes_out $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32, out_cap: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let bytes = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*);
            if bytes.len() > out_cap.max(0) as usize {
                return -1;
            }
            $crate::__host_import!(@fe $fe @write_bytes $x $c out_ptr, &bytes)
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident path_poll $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32, out_cap: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let poll = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*);
            match poll {
                $crate::pathfinder::PathPoll::Pending => 0,
                $crate::pathfinder::PathPoll::NoPath => -1,
                $crate::pathfinder::PathPoll::Path(cells) => {
                    let bytes = $crate::abi::path_cells_bytes(&cells);
                    if bytes.len() > out_cap.max(0) as usize {
                        return -2;
                    }
                    $crate::__host_import!(@fe $fe @write_bytes $x $c out_ptr, &bytes);
                    cells.len() as i32
                }
            }
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident path_opt $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32, out_cap: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let Some(cells) = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*) else {
                return -1;
            };
            let bytes = $crate::abi::path_cells_bytes(&cells);
            if bytes.len() > out_cap.max(0) as usize {
                return -2;
            }
            $crate::__host_import!(@fe $fe @write_bytes $x $c out_ptr, &bytes);
            cells.len() as i32
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident task_poll $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$id:ident,]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32, out_cap: i32,] {
            let poll = $crate::__host_import!(@fe $fe @host $x $c).$name($id);
            match poll {
                None => 0,
                Some(Err(e)) => {
                    $crate::__host_import!(@fe $fe @host $x $c).log(&format!("task {} failed: {e}", $id));
                    -1
                }
                Some(Ok(bytes)) => {
                    if bytes.len() > out_cap.max(0) as usize {
                        return -2;
                    }
                    $crate::__host_import!(@fe $fe @write_bytes $x $c out_ptr, &bytes);
                    bytes.len() as i32
                }
            }
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident event $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32, out_cap: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let Some((kind, name)) = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*) else {
                return 0;
            };
            let mut bytes = Vec::with_capacity(8 + name.len());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            if bytes.len() > out_cap.max(0) as usize {
                return -1;
            }
            $crate::__host_import!(@fe $fe @write_bytes $x $c out_ptr, &bytes);
            1
        });
    };
    (@emit [$fe:tt $x:tt] $name:ident anim $c:ident [$($raw:tt)*] [$($rd:tt)*]
        [$($arg:tt)*]) => {
        $crate::__host_import!(@fe $fe @register $x $c $name (i32) [$($raw)* out_ptr: i32, out_cap: i32,] {
            $( $crate::__host_import!(@read [$fe $x] $c $rd); )*
            let Some((anim, frame)) = $crate::__host_import!(@fe $fe @host $x $c).$name($($arg)*) else {
                return 0;
            };
            let mut bytes = Vec::with_capacity(12 + anim.len());
            bytes.extend_from_slice(&frame.to_le_bytes());
            bytes.extend_from_slice(&(anim.len() as u32).to_le_bytes());
            bytes.extend_from_slice(anim.as_bytes());
            if bytes.len() > out_cap.max(0) as usize {
                return -1;
            }
            $crate::__host_import!(@fe $fe @write_bytes $x $c out_ptr, &bytes);
            1
        });
    };

    // ---- trap stubs (real wasm signature, body traps) ----------------------
    (@trap $ctx:tt { $name:ident ($($params:tt)*) -> $ret:ident }) => {
        $crate::__host_import!(@trap_param $ctx $name $ret [] $($params)*);
    };
    (@trap_param $ctx:tt $name:ident $ret:ident [$($raw:tt)*] $p:ident : f64, $($rest:tt)*) => {
        $crate::__host_import!(@trap_param $ctx $name $ret [$($raw)* f64] $($rest)*);
    };
    (@trap_param $ctx:tt $name:ident $ret:ident [$($raw:tt)*] $p:ident : i32, $($rest:tt)*) => {
        $crate::__host_import!(@trap_param $ctx $name $ret [$($raw)* i32] $($rest)*);
    };
    (@trap_param $ctx:tt $name:ident $ret:ident [$($raw:tt)*] $p:ident : u32, $($rest:tt)*) => {
        $crate::__host_import!(@trap_param $ctx $name $ret [$($raw)* i32] $($rest)*);
    };
    // Every remaining parameter kind is a `ptr, len` pair.
    (@trap_param $ctx:tt $name:ident $ret:ident [$($raw:tt)*] $p:ident : $kind:ident,
        $($rest:tt)*) => {
        $crate::__host_import!(@trap_param $ctx $name $ret [$($raw)* i32 i32] $($rest)*);
    };
    (@trap_param $ctx:tt $name:ident unit [$($raw:tt)*]) => {
        $crate::__host_import!(@trap_emit $ctx $name (()) [$($raw)*]);
    };
    (@trap_param $ctx:tt $name:ident i32 [$($raw:tt)*]) => {
        $crate::__host_import!(@trap_emit $ctx $name (i32) [$($raw)*]);
    };
    (@trap_param $ctx:tt $name:ident f64 [$($raw:tt)*]) => {
        $crate::__host_import!(@trap_emit $ctx $name (f64) [$($raw)*]);
    };
    (@trap_param $ctx:tt $name:ident pair [$($raw:tt)*]) => {
        $crate::__host_import!(@trap_emit $ctx $name (i32) [$($raw)* i32]);
    };
    (@trap_param $ctx:tt $name:ident pair_opt [$($raw:tt)*]) => {
        $crate::__host_import!(@trap_emit $ctx $name (i32) [$($raw)* i32]);
    };
    (@trap_param $ctx:tt $name:ident triple [$($raw:tt)*]) => {
        $crate::__host_import!(@trap_emit $ctx $name (i32) [$($raw)* i32]);
    };
    (@trap_param $ctx:tt $name:ident triple_opt [$($raw:tt)*]) => {
        $crate::__host_import!(@trap_emit $ctx $name (i32) [$($raw)* i32]);
    };
    (@trap_param $ctx:tt $name:ident light [$($raw:tt)*]) => {
        $crate::__host_import!(@trap_emit $ctx $name (i32) [$($raw)* i32]);
    };
    // Every remaining return kind is an `out_ptr, out_cap` buffer returning `i32`.
    (@trap_param $ctx:tt $name:ident $ret:ident [$($raw:tt)*]) => {
        $crate::__host_import!(@trap_emit $ctx $name (i32) [$($raw)* i32 i32]);
    };
    (@trap_emit [$fe:tt $x:tt] $name:ident ($ret:ty) [$($raw:tt)*]) => {
        $crate::__host_import!(@fe $fe @register_trap $x $name ($ret) [$($raw)*]);
    };
}

/// A wasm value type in a host import's signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WasmTy {
    I32,
    F64,
}

/// How one host-import parameter crosses the guest boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    I32,
    F64,
    U32,
    Str,
    Bytes,
    BytesOwned,
    F32s,
    U32s,
}

impl ParamKind {
    /// The wasm params this kind occupies.
    pub fn wasm(self) -> &'static [WasmTy] {
        match self {
            ParamKind::I32 | ParamKind::U32 => &[WasmTy::I32],
            ParamKind::F64 => &[WasmTy::F64],
            ParamKind::Str
            | ParamKind::Bytes
            | ParamKind::BytesOwned
            | ParamKind::F32s
            | ParamKind::U32s => &[WasmTy::I32, WasmTy::I32],
        }
    }
}

/// How a host import's result reaches the guest (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetKind {
    Unit,
    I32,
    F64,
    Json,
    Pair,
    PairOpt,
    Triple,
    TripleOpt,
    Light,
    F32s,
    BytesOut,
    PathPoll,
    PathOpt,
    TaskPoll,
    Event,
    Anim,
}

impl RetKind {
    /// The trailing out-buffer wasm params this kind appends.
    pub fn out_params(self) -> &'static [WasmTy] {
        match self {
            RetKind::Unit | RetKind::I32 | RetKind::F64 => &[],
            RetKind::Pair
            | RetKind::PairOpt
            | RetKind::Triple
            | RetKind::TripleOpt
            | RetKind::Light => &[WasmTy::I32],
            RetKind::Json
            | RetKind::F32s
            | RetKind::BytesOut
            | RetKind::PathPoll
            | RetKind::PathOpt
            | RetKind::TaskPoll
            | RetKind::Event
            | RetKind::Anim => &[WasmTy::I32, WasmTy::I32],
        }
    }

    /// The wasm result type (`None` for `unit`).
    pub fn wasm_result(self) -> Option<WasmTy> {
        match self {
            RetKind::Unit => None,
            RetKind::F64 => Some(WasmTy::F64),
            _ => Some(WasmTy::I32),
        }
    }
}

/// A runtime backend that exposes (a subset of) the host-import table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The wasmi + wasmtime foreground runtimes.
    Native,
    /// Browser-native `WebAssembly` (trusted guests).
    Web,
    /// The untrusted SAB `Worker` runtime.
    Worker,
    /// The background worker guest (Tier 3), real import.
    Tier3,
    /// The background worker guest (Tier 3), registered as a trap stub.
    Tier3Trap,
}

/// One named, typed host-import parameter.
#[derive(Clone, Copy, Debug)]
pub struct HostParam {
    pub name: &'static str,
    pub kind: ParamKind,
}

/// A host import described by the table.
#[derive(Clone, Copy, Debug)]
pub struct HostImport {
    pub name: &'static str,
    pub params: &'static [HostParam],
    pub ret: RetKind,
    pub backends: &'static [Backend],
}

impl HostImport {
    /// Whether `backend` exposes this import.
    pub fn on(&self, backend: Backend) -> bool {
        self.backends.contains(&backend)
    }

    /// The flattened wasm parameter types (including trailing out-buffer params).
    pub fn wasm_params(&self) -> Vec<WasmTy> {
        self.params
            .iter()
            .flat_map(|p| p.kind.wasm().iter().copied())
            .chain(self.ret.out_params().iter().copied())
            .collect()
    }

    /// The wasm result type (`None` for `unit`).
    pub fn wasm_result(&self) -> Option<WasmTy> {
        self.ret.wasm_result()
    }

    /// A WAT `(import ..)` declaration for this import under `module` (for
    /// building link-coverage test fixtures).
    pub fn wat_import(&self, module: &str) -> String {
        let ty = |t: WasmTy| match t {
            WasmTy::I32 => "i32",
            WasmTy::F64 => "f64",
        };
        let params: Vec<&str> = self.wasm_params().into_iter().map(ty).collect();
        let result = self.wasm_result().map(|t| format!(" (result {})", ty(t))).unwrap_or_default();
        format!(
            "(import \"{module}\" \"{}\" (func (param {}){result}))",
            self.name,
            params.join(" ")
        )
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __host_import_descriptors {
    ($( $name:ident ( $($pn:ident : $pk:ident),* $(,)? ) -> $ret:ident [ $($be:ident)* ] ; )*) => {
        &[$(
            $crate::abi_manifest::HostImport {
                name: stringify!($name),
                params: &[$(
                    $crate::abi_manifest::HostParam {
                        name: stringify!($pn),
                        kind: $crate::__host_import_descriptors!(@param $pk),
                    }
                ),*],
                ret: $crate::__host_import_descriptors!(@ret $ret),
                backends: &[$( $crate::__host_import_descriptors!(@backend $be) ),*],
            }
        ),*]
    };
    (@param i32) => { $crate::abi_manifest::ParamKind::I32 };
    (@param f64) => { $crate::abi_manifest::ParamKind::F64 };
    (@param u32) => { $crate::abi_manifest::ParamKind::U32 };
    (@param str) => { $crate::abi_manifest::ParamKind::Str };
    (@param bytes) => { $crate::abi_manifest::ParamKind::Bytes };
    (@param bytes_owned) => { $crate::abi_manifest::ParamKind::BytesOwned };
    (@param f32s) => { $crate::abi_manifest::ParamKind::F32s };
    (@param u32s) => { $crate::abi_manifest::ParamKind::U32s };
    (@ret unit) => { $crate::abi_manifest::RetKind::Unit };
    (@ret i32) => { $crate::abi_manifest::RetKind::I32 };
    (@ret f64) => { $crate::abi_manifest::RetKind::F64 };
    (@ret json) => { $crate::abi_manifest::RetKind::Json };
    (@ret pair) => { $crate::abi_manifest::RetKind::Pair };
    (@ret pair_opt) => { $crate::abi_manifest::RetKind::PairOpt };
    (@ret triple) => { $crate::abi_manifest::RetKind::Triple };
    (@ret triple_opt) => { $crate::abi_manifest::RetKind::TripleOpt };
    (@ret light) => { $crate::abi_manifest::RetKind::Light };
    (@ret f32s) => { $crate::abi_manifest::RetKind::F32s };
    (@ret bytes_out) => { $crate::abi_manifest::RetKind::BytesOut };
    (@ret path_poll) => { $crate::abi_manifest::RetKind::PathPoll };
    (@ret path_opt) => { $crate::abi_manifest::RetKind::PathOpt };
    (@ret task_poll) => { $crate::abi_manifest::RetKind::TaskPoll };
    (@ret event) => { $crate::abi_manifest::RetKind::Event };
    (@ret anim) => { $crate::abi_manifest::RetKind::Anim };
    (@backend native) => { $crate::abi_manifest::Backend::Native };
    (@backend web) => { $crate::abi_manifest::Backend::Web };
    (@backend worker) => { $crate::abi_manifest::Backend::Worker };
    (@backend tier3) => { $crate::abi_manifest::Backend::Tier3 };
    (@backend tier3_trap) => { $crate::abi_manifest::Backend::Tier3Trap };
}

/// Every host import in the table, in table order.
pub const HOST_IMPORTS: &[HostImport] =
    crate::for_each_host_import!([crate::__host_import_descriptors] {});

/// Iterate the imports a backend exposes.
pub fn imports_for(backend: Backend) -> impl Iterator<Item = &'static HostImport> {
    HOST_IMPORTS.iter().filter(move |i| i.on(backend))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn names(backend: Backend) -> HashSet<&'static str> {
        imports_for(backend).map(|i| i.name).collect()
    }

    #[test]
    fn names_are_unique() {
        let mut seen = HashSet::new();
        for import in HOST_IMPORTS {
            assert!(seen.insert(import.name), "duplicate host import `{}`", import.name);
        }
    }

    #[test]
    fn backend_subset_sizes() {
        assert_eq!(imports_for(Backend::Native).count(), 110);
        assert_eq!(imports_for(Backend::Web).count(), 110);
        assert_eq!(imports_for(Backend::Worker).count(), 110);
        assert_eq!(imports_for(Backend::Tier3).count(), 23);
        assert_eq!(imports_for(Backend::Tier3Trap).count(), 23);
    }

    #[test]
    fn web_and_worker_match_native() {
        let native = names(Backend::Native);
        assert_eq!(names(Backend::Web), native);
        assert_eq!(names(Backend::Worker), native);
        assert!(names(Backend::Tier3Trap).is_subset(&native));
        assert!(names(Backend::Tier3).is_disjoint(&names(Backend::Tier3Trap)));
    }

    #[test]
    fn wasm_signatures_flatten_kinds() {
        let find = |name| HOST_IMPORTS.iter().find(|i| i.name == name).unwrap();
        let pick_at = find("pick_at");
        assert_eq!(
            pick_at.wasm_params(),
            [WasmTy::F64, WasmTy::F64, WasmTy::I32, WasmTy::I32, WasmTy::I32, WasmTy::I32]
        );
        assert_eq!(pick_at.wasm_result(), Some(WasmTy::I32));
        assert_eq!(find("get_pos").wasm_params(), [WasmTy::I32; 3]);
        assert_eq!(find("height_at").wasm_result(), Some(WasmTy::F64));
        assert_eq!(find("log").wasm_result(), None);
        assert_eq!(
            find("height_at").wat_import("env"),
            r#"(import "env" "height_at" (func (param f64 f64) (result f64)))"#
        );
    }
}
