# Status Bar Layout Specification

**Target:** ironbar + `mango-bard` on Hyprland (Arch Linux), bar `position: top`
**Scope:** module ordering and layout invariants only — not styling, colours, or fonts
**Status:** normative (MUST / SHOULD / MAY per RFC 2119)

The bar is generated, never handwritten. `mango-bard gen-config` builds
`~/.config/ironbar/config.json` from `src/ironbar/bard/src/genconfig.rs`; this
spec constrains what that generator produces, in ironbar's own `start` /
`center` / `end` rows (ironbar has no separate `modules-left/center/right`
naming — see [bar.md](bar.md) for the module-to-file map and the daemon's own
CLI).

---

## 1. Layout

```
┌────────────────────────────────────────────────────────────────────────────────────────────┐
│ ⟡ 1 2 3 4 5 6 7 8 9 ov  window title…   12:34 Tue 25/08 🍅25:00   ▪▪▪ CPU RAM  DOK BAT ◐AI  [⋯]☕ ♫ 🔊 🔇  📶 🖧 🔒 📡 ⛺ ᛒ ⏻ │
└────────────────────────────────────────────────────────────────────────────────────────────┘
  LAUNCH  TAGS            FOCUS               TIME              TRAY  RESOURCES        TOOLS  AUDIO   CONNECTIVITY        SESSION
  ─────────────────────────►                 CENTER              ◄──────────────────────────────────────────────────────
        start                                center                                     end
```

| Row | Block | Order (outward from anchor) |
| --- | --- | --- |
| `start` | launcher → tags → focus | `spark` → `ws-1…9` + `ws-ov` → `win` |
| `center` | time | `clock` → `date` → `pomo` |
| `end` | tray → resources → tools → audio → connectivity → session | `tray` → `cpu` `memory` `docker` `battery` `claudebar` → tools-drawer(`colorpicker` `darkmode` `snip`) `inhibit` → `music` `volume` `mic` → `net-spinner` `wifi` `eth` `netsec` `hotspot` `remote` `bluetooth` → `power` |

The fallback bar (no monitor matched, so no tag block — see §3.1) carries the
same blocks minus `ws-*`.

---

## 2. Invariants

The hard constraints. Every ordering decision in §3 follows from them.

### INV-1 — Zero reflow

No module MAY change the on-screen X position of any other module.

Variable-width modules MUST be placed at the **growth end** of their block:

- `start`'s tags block has a fixed member count (§INV-4); `win` is the last
  module in `start` because it is the one truly unbounded field (a window
  title has no natural cap) — it grows rightward into free space, past every
  fixed-width neighbour, and displaces nothing to its left.
- `end` grows leftward from the screen edge → `tray` is **first** (leftmost)
  in `end`, so a tray icon appearing or disappearing moves only the tray.
- `music` sits mid-block (audio), not at a growth end, so it MUST be
  fixed-width by truncation (§3.4), not by position.

### INV-2 — Corner bleed

The leftmost and rightmost modules MUST occupy the physical corner pixel.

- The bar's own `margin` MUST be `0` on every side (ironbar `MarginConfig`
  defaults to 0; `genconfig.rs` MUST NOT set it).
- The `start` and `end` row containers MUST carry no margin or padding on
  their outward side, and MUST NOT round their outward corner — `#bar #start`
  squares its top-left corner, `#bar #end` its top-right. `#bar #center` keeps
  its full rounding; it never touches a screen edge.
- `spark` and `power` carry their own inset as `padding` (inside the
  clickable button), never as `margin` on the row or the module — margin is
  dead space and forfeits the corner target outright.

### INV-3 — No destructive corner action

A module in a corner MUST NOT fire an irreversible action on click. Corner
targets are hit accidentally by fast pointer movement.

- `power` is permitted in the top-right corner only because `on_click_left`
  opens `powermenu.sh` (a menu), never a direct shutdown/logout.

### INV-4 — Tag row at constant X

The 9 workspace tags plus the overview pill are muscle-memory targets. Their
absolute X MUST be constant across all states.

- Only `spark` MAY precede them in `start`.
- All 9 numbered tags MUST render at all times (`show_if: "#<slug>_tags"`
  gates the *group*, not individual tags — see `mango.rs::var_tags`).
- Every tag button MUST have identical fixed width (`.ws button`,
  `.ws button label`, both `min-width: 24px`) — a single- and a double-digit
  label MUST NOT differ, and neither MUST the overview pill's own toggle.

### INV-5 — Fixed-width numerics

Any module rendering a changing **digit string** — `cpu`, `memory`,
`battery`, `claudebar` (percentages, 1-3 digits), `docker` (a container
count with no digit cap — checked against `docker.rs`, not assumed from a
typical dev-box count) and `clock`/`date`/`pomo` (fixed-format, but
proportional digit widths still shimmer on substitution) — MUST render at
constant width.

- `font-feature-settings: "tnum" 1` (tabular figures) on both the module's
  own node and its `label` descendant (the T17 GTK4 inheritance trap: a
  direct match beats an inherited value, so a class rule on the container
  never reaches the label's own font properties).
- `min-width` on the module's own node **only**. This is a *reserved width*,
  not a *centred* width (contrast INV-4's `.ws`, the one module in this file
  that also puts `min-width` on its `button`/`label` descendants): content
  packs at the module's natural (left) edge — GtkButton's `hexpand: false`
  and the T19/T20 GTK4 trap below both hold here too, just used on purpose
  instead of fought — and slack accumulates on the trailing edge. Set the
  reserve to the *realistic* max content width plus one measured digit's
  worth of slack (not the absolute max): a rarer, one-digit-wider string
  then eats the slack and stays inside the box instead of reflowing. A
  `label` min-width twin here would be wrong, not just redundant — GtkLabel
  defaults `xalign: 0.5`, so it would centre the text in the reserve and
  walk the icon sideways on every digit-count change (T25).
- Format padding (e.g. `{usage:>3}%`) MAY be used in addition, never instead.

`volume` and `mic` are a different case, not this invariant: T9 reduced both
to an icon-only, discrete-state display (no digit string at all — the
percentage moved to the hover popup), so `tnum` is a no-op there. Their own
reflow risk is INV-1's, not INV-5's: a two-state width jump (icon vs. empty),
closed with a plain `min-width` and no tabular-figure feature needed.

This is the invariant `center` depends on most directly: `clock`/`date`/`pomo`
sit inside a group GTK centres as one block (§INV-6), so an unpadded digit
change there does not just wobble locally, it shifts the whole block sideways
on every screen tall enough to notice.

### INV-6 — Constant-width center

`center` MUST have constant total width, or centering visibly wobbles.

- `clock`/`date` format MUST be fixed-width.
- `pomo` MUST render a same-width placeholder when idle (`--:--`), never
  collapse to zero.

### Two GTK4 traps that make INV-4/INV-5 actually hold

Paid for once already in this repo (IRONBAR.md T19/T20/T21/T25); restated
here because a future edit to this bar will hit them again if it doesn't
know:

- **Sizing a `button`'s container alone does not size its child `label`** —
  the button packs the label at natural width, start-aligned regardless of
  the container's own `min-width`, and `justify: center` only aligns Pango
  lines against each other, not the layout inside the widget. INV-4's `.ws`
  wants this *fixed*: both `.ws button` and `.ws button label` carry the
  same `min-width`, so the label fills the reserve and its own `xalign: 0.5`
  centres the text. INV-5's pills want the opposite — content packed at one
  edge, slack at the other — so they deliberately give `min-width` to the
  module's own node only and stop there; adding the `label` twin here would
  re-introduce the centring this list exists to warn against.
- **A bare class beats `#bar #end > *` in this engine.** Live-verified
  (T21): the opposite of the W3C specificity model. Any per-pill override
  (padding, width) MUST be written as a bare class rule, never assumed to
  win by qualifying it under `#bar #end`.

---

## 3. Module specification

### 3.1 `start` — launcher, tags, focus

| # | Module | Interaction | Width | Rationale |
| - | --- | --- | --- | --- |
| 1 | `spark` | click → `rofi -show drun` | fixed | Highest-frequency click in the bar; occupies the top-left magic corner (INV-2). |
| 2 | `ws-1…9`, `ws-ov` | click / scroll / right-click popup | fixed | Second-highest click frequency, shortest travel from the corner, constant X (INV-4). |
| 3 | `win` | none (scroll: brightness) | variable | Highest-variance width in the bar; last in `start` so it displaces nothing (INV-1). Truncated at 32 chars (`truncate.max_length`), never a click target beyond its own scroll. |

**Semantic reading order:** left to right, *identity → location → focus*
("what can I launch / where am I / what am I doing").

The fallback bar (no monitor name to build tag vars from) renders `spark` and
`win` with no tag block — see `build()`'s own doc comment for why an unlisted
output still gets a bar rather than none.

### 3.2 `center` — time

| # | Module | Interaction | Width | Rationale |
| - | --- | --- | --- | --- |
| 4 | `clock` | none (hover popup: world clocks) | fixed | Highest glance frequency, weakest click need — center is the shortest saccade from screen-centre gaze. |
| 5 | `date` | click → calendar app; hover popup: month | fixed | Same semantic domain as clock. |
| 6 | `pomo` | left: start/pause; right: mute; hover popup | fixed | Click frequency is a handful per day; the weak click target this placement costs is acceptable at that frequency. |

Center placement, not the right edge, because glance-only content minimises
eye travel — this matters more the wider the monitor, and costs nothing since
none of the three needs a corner-quality click target.

### 3.3 `end`, block 1 — tray

| # | Module | Interaction | Width | Rationale |
| - | --- | --- | --- | --- |
| 7 | `tray` | click → app menus | variable | Leads `end` so its own growth is absorbed by itself alone (INV-1) — the only variable-width member of `end`. |

### 3.4 `end`, block 2 — resources

`cpu`, `memory`, `docker`, `battery`, `claudebar` — one glance-only ambient
group, hover popup per pill, click launches an external tool (`btop`,
`powermode.sh`, `powertop`) or, for docker, a right-click menu.

Grouped by **proximity/common region**: these five answer one question — "is
this machine healthy" — read in one saccade rather than five separate stops.
Order within the block is itself a reading of load, from general to specific:
cpu → memory → docker (processes) → battery (power) → claudebar (an external
budget, least tied to the machine itself).

### 3.5 `end`, block 3 — tools

Click-only, never-glanced. Per **Hick's Law**, choice/scan time grows with
the number of visible options — four permanent icons cost attention on every
scan for something used a handful of times a day.

- **`tools` drawer** — one `custom` module: a permanent trigger button plus
  `colorpicker`, `darkmode`, `snip`, each gated `show_if: "#tools_open"`.
  `on_mouse_enter`/`on_mouse_exit` on the trigger set `tools_open` directly
  (`ironbar var set tools_open true|false`) — bar-local UI state with no
  daemon collector behind it, so it does not go through `mango-bard`.
  `transition_type: "slide_start"` (ironbar's schema offers
  `none`/`crossfade`/`slide_start`/`slide_end` only — no slide-from-top).
- **`inhibit`** — kept **outside** the drawer, immediately to its right,
  exercising this section's own exception: keep-awake is a state you need to
  glance at, not an action you go looking for. It renders at constant width
  in both states (same-size `nf-md-coffee`/`nf-md-coffee_outline` glyphs).

Ironbar 0.19 has no `group`/`drawer` primitive (waybar's own `group` +
`"drawer"` has no counterpart) — `show_if` + `transition_type` +
`on_mouse_enter`/`on_mouse_exit` is the mechanism this ironbar version
provides for a reveal-on-hover group.

### 3.6 `end`, block 4 — audio

`music`, `volume`, `mic` — one domain by proximity/common region: what's
playing, and what the machine is doing with sound.

- **`music`** — MPRIS, truncated to 30 chars (`truncate.max_length`) so it
  cannot grow past its slot; it sits mid-block, not at a growth end, so
  INV-1 requires this fixed cap rather than open-ended width.
  - **T26 deviation:** the CSS carried a `min-width: 220px` reserve against
    that cap. Live measurement found real track titles never approach 30
    chars, so the reserve sat empty behind whatever short title (or
    nothing) was actually showing — the largest gap on the bar. The
    reserve is removed; `volume`/`mic` now shift when a track starts or
    ends. That is a discrete, user-caused event, not the per-tick reflow
    INV-1 guards against, so it is accepted rather than reserved against.
- **`volume`** — icon only on the bar (percentage moved to the popup, T9);
  left click opens a 5s mixer, right the full mixer, scroll adjusts.
- **`mic`** — mute-state icon, renders empty when unmuted so the common case
  contributes no width.

### 3.7 `end`, block 5 — connectivity

`net-spinner`, `wifi`, `eth`, `netsec`, `hotspot`, `remote`, `bluetooth` — one
block by proximity/common region: "how am I connected, and is it safe" is a
single question with seven possible answers, not seven separate questions.
This is the layout's largest single consolidation, so the internal order is
itself part of the spec, general to specific:

1. `net-spinner` — busy indicator, replaces `wifi` in place while a link
   transitions (`show_if: "#net_busy"`, logical inverse of `wifi`'s own
   `show_if`).
2. `wifi` — link state and signal.
3. `eth` — wired link state.
4. `netsec` — the security verdict *for* the link `wifi`/`eth` just
   described (DNS leak, route conflict, captive portal, open network) —
   immediately after the links it grades, not detached from them.
5. `hotspot` — a *mode* of the same wifi radio `wifi` already covers;
   `show_if`-hidden until active, so it costs nothing when unused.
6. `remote` — inbound reachability (wayvnc/KDE Connect), the same
   "who can reach this machine" question the security pill just answered,
   read outward from it rather than lumped into `netsec` itself (different
   protocol family, own click target).
7. `bluetooth` — nearest remaining connectivity radio; last in the block
   because it is the one native module with click-to-toggle-popup that must
   sit next to `power`'s own corner treatment (T9: its popup does not
   respect `popup_autohide`, closed instead via unconditional
   `hover_exit`/`hide_popup` — see `bluetooth_module()`'s own doc comment).

### 3.8 `end`, block 6 — session

| # | Module | Interaction | Width | Rationale |
| - | --- | --- | --- | --- |
| 18 | `power` | click → **menu** (`powermenu.sh`) | fixed | Top-right magic corner; menu-only per INV-3. |

---

## 4. Permitted variant

Not currently taken. If the power menu is used rarely and volume is adjusted
by scrolling on the bar, `volume` and `power` MAY be swapped so `volume`
occupies the top-right corner — a scroll target in a magic corner is the
fastest possible volume control (fling, scroll, done, no click, no precision),
and strengthens INV-3 further since scroll-to-adjust is non-destructive.

No other reordering is specified. Moving the clock off `center` is explicitly
**out of scope**.

---

## 5. Reference implementation (ironbar / `mango-bard`)

Config is generated, not handwritten — see `src/ironbar/bard/src/genconfig.rs`.
Shape (module bodies omitted; see the file itself for the full JSON each
builder returns):

```rust
// genconfig.rs::build(), per real monitor bar
"start":  start_modules(&bar_name),   // spark, workspace_pills(), win
"center": time_modules(&bar_name),    // clock, date, pomo
"end":    end_modules(&bar_name),     // tray, resource_modules(), tools_modules(),
                                       // audio_modules(), net_modules(), hotspot,
                                       // remote, bluetooth, power
```

```rust
fn start_modules(bar_name: &str) -> Vec<Value> {
    let mut m = vec![spark_module()];
    m.extend(workspace_pills(mon, slug));   // INV-4
    m.push(window_module(bar_name));        // INV-1: last, variable width
    m
}

fn end_modules(bar_name: &str) -> Vec<Value> {
    let mut end = vec![tray_module()];      // INV-1: leads end, variable width
    end.extend(resource_modules(bar_name)); // cpu, memory, docker, battery, claudebar
    end.extend(tools_modules(bar_name));    // tools drawer, inhibit
    end.extend(audio_modules(bar_name));    // music, volume, mic
    end.extend(net_modules(bar_name));      // net-spinner, wifi, eth, netsec
    end.push(hotspot_module(bar_name));
    end.push(remote_module(bar_name));
    end.push(bluetooth_module(bar_name));
    end.push(power_module());
    end
}
```

The tools drawer:

```rust
fn tools_modules(bar_name: &str) -> Vec<Value> {
    vec![
        json!({
            "type": "custom", "name": "tools", "class": "tools",
            "bar": [ /* trigger button + gated children */ ],
            "on_mouse_enter": "ironbar var set tools_open true",
            "on_mouse_exit": "ironbar var set tools_open false"
        }),
        // colorpicker_module()/darkmode_module()/snip_module(), each with
        // "show_if": "#tools_open", "transition_type": "slide_start"
        inhibit_module(bar_name),  // outside the drawer, INV-glanceable exception
    ]
}
```

CSS (`src/matugen/templates/ironbar/style.css`; selectors are classes set by
`genconfig.rs`, not waybar-style `#custom-*` ids):

```css
/* INV-2 — corner bleed: the outward corner squared and flush, inset moved
   inside the button as padding, never margin. */
#bar #start { border-radius: 0 12px 12px 0; margin: 0 0 4px 0; padding: 0; }
#bar #end   { border-radius: 12px 0 0 12px; margin: 0 0 4px 0; padding: 0; }
.spark { padding: 0 10px; }
.power { padding: 0 10px; }

/* INV-4 — identical width for every tag, both nodes (GTK4 trap above) */
.ws button, .ws button label { min-width: 24px; }

/* INV-5 — tabular figures on both nodes (T17 trap), min-width on the
   module's own node ONLY — a reserve, not a centred box (see §2's own
   distinction from INV-4's `.ws`). volume/mic are icon-only (T9) —
   min-width alone, no tnum, per §2's own distinction. */
.cpu, .memory, .docker, .battery, .claudebar, .clock, .date, .pomo,
.cpu label, .memory label, .docker label, .battery label, .claudebar label,
.clock label, .date label, .pomo label {
  font-feature-settings: "tnum" 1;
}
.cpu, .memory { min-width: 58px; }  /* realistic max + 1 digit slack, T25 */
.docker { min-width: 45px; }
.battery { min-width: 80px; }
.claudebar { min-width: 145px; }
.clock { min-width: 50px; }
.date  { min-width: 84px; }
.pomo  { min-width: 52px; }
.volume, .mic { min-width: 20px; }
.music { min-width: 220px; } /* INV-1: reserves music's own truncated cap */
```

---

## 6. Verification checklist

Width/position claims are verified by **pixel-column analysis of a `grim`
crop, never by eye** (T19 measured "pixel-perfect" by eye and was 7.5px out).
Always pass `grim -o <output>` — a bare `grim` captures every monitor and
whatever else is on screen.

- [ ] Fling pointer to top-left at full speed → `spark` activates on click (no dead pixel).
- [ ] Fling pointer to top-right at full speed → `power` **menu** opens; no action fires, no state changes.
- [ ] Start/quit a tray application → no module other than the tray changes position.
- [ ] Focus a window with a 100-character title → tags and `spark` do not move.
- [ ] `cpu`/`memory`/`docker`/`battery`/`claudebar` each transition realistic
      max → rare overflow (99%→100%, 99→999, 100%→100%+eco leaf, with/without
      the stale-cache marker) → `tray` (leftmost in `end`) does not shift by
      a single pixel. `center` is the wrong subject for these five: they
      moved from `start` to `end` at T23, and `end` is right-anchored, so
      growth pushes `end`'s own leftmost member, `tray` (T25).
- [ ] `pomo` idle → running → idle: `center` width unchanged.
- [ ] All 9 tags visible with every tag empty; `ws-ov` toggled on and off with no shift in `wifi`'s position.
- [ ] Tag 9 → overview → tag 1 transition: no button width change.
- [ ] Hover the `tools` trigger → `colorpicker`/`darkmode`/`snip` expand; move the pointer across them, confirm no collapse; unhover → collapses; `end` returns to identical positions.
- [ ] `hotspot` toggled active → inactive: neighbouring `remote`/`bluetooth` do not shift.
- [ ] A long MPRIS track title playing → `music` truncates at 30 chars, `volume` does not move.

---

## Sources

Layout derived from the following. No controlled A/B testing exists for
status-bar module ordering specifically; the empirical basis is Fitts's law
and corner/edge target research for the outer anchors, and Gestalt grouping
plus Hick's Law for the interior blocks §3 introduces.

- Fitts's law, "rule of the infinite edges" and magic corners — https://en.wikipedia.org/wiki/Fitts%27s_law
- Tognazzini / Atwood on corner pinning action and infinite width — https://blog.codinghorror.com/fitts-law-and-infinite-width/
- Non-destructive corner actions (Apple's constraint) — https://leancrew.com/all-this/2018/10/a-big-target/
- Nielsen Norman Group, Fitts's Law and its applications in UX — https://www.nngroup.com/articles/fitts-law/
- Fitts's law 2D extension (MacKenzie & Buxton 1992), cited in — https://www.nngroup.com/articles/fitts-law/
- Nielsen Norman Group, the Gestalt principle of proximity — https://www.nngroup.com/articles/gestalt-proximity/
- Interaction Design Foundation, Gestalt principles including common region — https://ixdf.org/literature/topics/gestalt-principles
- Nielsen Norman Group, chunking for scannability — https://www.nngroup.com/articles/chunking/
- Hick's Law (choice reaction time grows with option count) — https://en.wikipedia.org/wiki/Hick%27s_law
- Waybar module groups, `modules-left/center/right` semantics (prior bar, superseded) — https://wiki.hypr.land/Useful-Utilities/Status-Bars/
- Polybar tray-as-module positioning (cross-check on tray growth behaviour) — https://polybar.readthedocs.io/en/stable/user/modules/tray.html
- ironbar's own module/config reference — https://github.com/JakeStanger/ironbar
- `IRONBAR.md` T19–T22 — the two GTK4 traps (§2) and the T22 `center`-emptying decision this spec's §1/§3.1 deliberately reverses (see IRONBAR.md's T23 entry for why the reversal is safe).

Generated assets embedded in this document: the ASCII bar diagram (§1), the
Rust/CSS reference implementation excerpts (§5) are authored inline in this
file — no external generator or build step, no separate source artifact
exists.
