# Status Bar Layout Specification

**Target:** ironbar + `mango-bard` on mango (Arch Linux), bar `position: top`
**Scope:** module ordering and layout invariants only. Not styling, colours,
or fonts.
**Status:** normative (MUST / SHOULD / MAY per RFC 2119)

The generator makes the bar. Nobody writes the bar by hand.
`mango-bard gen-config` builds `~/.config/ironbar/config.json` from
`ironbar/bard/src/genconfig.rs`. This spec constrains what that generator
puts in ironbar's own `start`, `center` and `end` rows. Ironbar has no
separate `modules-left/center/right` naming. See [bar.md](bar.md) for the
module-to-file map and the daemon's own CLI.

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
| `end` | tray → resources → tools → audio → connectivity → session | `tray` `traytoggle` `keepass` → `sysload`(`cpu`+`memory` rows) `battery` `devload`(`claude` row, `docker`+`archupdate` row) → `colorpicker` `darkmode` `snip` `inhibit` → `music` `volume` `mic` → `net-spinner` `wifi` `eth` `netsec` `hotspot` `remote` `bluetooth` → `power` |

The fallback bar carries the same blocks, minus `ws-*`. That bar matched no
monitor, thus it has no tag block. See §3.1.

---

## 2. Invariants

The hard constraints. Every ordering decision in §3 follows from them.

### INV-1 — Zero reflow

No module MAY change the on-screen X position of any other module.

The generator MUST put a variable-width module at the **growth end** of its
block:

- `start`'s tags block has a fixed member count (§INV-4). `win` is the last
  module in `start`, because it is the one field with no bound. A window
  title has no natural cap. `win` grows rightward into free space, past
  every fixed-width neighbour. It moves nothing on its left.
- `end` grows leftward from the screen edge. Thus `tray` is **first**
  (leftmost) in `end`. A tray icon that appears or disappears moves only
  the tray.
- `music` sits in the middle of the audio block, not at a growth end. Thus
  `music` MUST hold a fixed width by truncation (§3.4), not by position.

### INV-2 — Corner bleed

The leftmost and rightmost modules MUST occupy the physical corner pixel.

- The bar's own `margin` MUST be `0` on every side. Ironbar's
  `MarginConfig` defaults to 0. `genconfig.rs` MUST NOT set it.
- The `start` and `end` row containers MUST carry no margin or padding on
  their outward side. They MUST NOT round their outward corner.
  `#bar #start` squares its top-left corner. `#bar #end` squares its
  top-right corner. `#bar #center` keeps its full rounding, because it
  never touches a screen edge.
- `spark` and `power` carry their own inset as `padding`, inside the
  clickable button. They never carry it as `margin` on the row or on the
  module. Margin is dead space, and it loses the corner target completely.

### INV-3 — No destructive corner action

A module in a corner MUST NOT fire an irreversible action on click. Fast
pointer movement hits corner targets by accident.

- `power` is permitted in the top-right corner for one reason only. Its
  `on_click_left` opens the `powermenu.sh` menu. It never starts a direct
  shutdown or logout.

### INV-4 — Tag row at constant X

The user hits the 9 workspace tags and the overview pill from memory. Their
absolute X MUST be constant in all states.

- Only `spark` MAY come before them in `start`.
- All 9 numbered tags MUST render at all times. `show_if: "#<slug>_tags"`
  gates the *group*, not the individual tags. See `mango.rs::var_tags`.
- Every tag button MUST have the same fixed width. `.ws button` and
  `.ws button label` both carry `min-width: 24px`. A single-digit label
  and a double-digit label MUST NOT differ in width. The overview pill's
  own toggle MUST NOT differ either.

### INV-5 — Fixed-width numerics

Any module that renders a changing **digit string** MUST render at constant
width. `battery` shows a percentage. `sysload` holds the `cpu` and `memory`
rows. Since T28 these two rows show an icon and a `.gauge` only, with no
digit of their own. The reserve still guards the icon and gauge envelope.

`devload` holds the `claude` row's percentage and countdown, and the
`docker` and `archupdate` cells. The container count has no digit cap.
`docker.rs` gives the checked value, in place of an assumed dev-box count.
`clock`, `date` and `pomo` use a fixed format. Their proportional digit
widths still shimmer on substitution.

- Set `font-feature-settings: "tnum" 1` (tabular figures) on the module's
  own node and on its `label` descendant. This is the T17 GTK4 inheritance
  trap: a direct match beats an inherited value. Thus a class rule on the
  container never reaches the label's own font properties.
- Set `min-width` on the module's own node **only**. This is a *reserved
  width*, not a *centred* width. INV-4's `.ws` is the contrast: it is the
  one module in this file that also puts `min-width` on its `button` and
  `label` descendants. The content packs at the module's natural (left)
  edge, and the slack collects on the trailing edge. GtkButton's
  `hexpand: false` and the T19/T20 GTK4 trap below both apply here too,
  but this rule uses them on purpose.
- Set the reserve to the *realistic* maximum content width, plus one
  measured digit of slack. Do not use the absolute maximum. A rarer string
  that is one digit wider then uses the slack and stays inside the box, in
  place of a reflow. A `label` min-width twin here would be wrong, not only
  unnecessary. GtkLabel defaults `xalign: 0.5`, so the twin would centre
  the text in the reserve and move the icon sideways at every change of
  digit count (T25).
- Format padding (for example `{usage:>3}%`) MAY be used in addition. Do
  not use it in place of the reserve.

`battery` and `devload` are a known exception. The user shrank both reserves
below their realistic maximum width. A future pass MUST NOT restore the old,
larger `min-width` values without a request from the user.

`volume` and `mic` are a different case. This invariant does not apply to
them. T9 changed both to an icon-only display with discrete states. They
show no digit string, because the percentage moved to the hover popup. Thus
`tnum` does nothing there.

Their reflow risk belongs to INV-1, not to INV-5. The risk is a two-state
width jump between an icon and an empty label. A plain `min-width` closes
that jump. The tabular-figure feature is not necessary.

`center` depends on this invariant most directly. GTK centres `clock`,
`date` and `pomo` as one block (§INV-6). Thus an unpadded digit change there
does not only wobble in place. It moves the whole block sideways on every
screen tall enough to notice.

### INV-6 — Constant-width center

`center` MUST have a constant total width. If it does not, the centring
wobbles visibly.

- The `clock` and `date` format MUST be fixed-width.
- `pomo` MUST render a placeholder of the same width when idle (`--:--`).
  It MUST NOT collapse to zero width.

### Two GTK4 traps that make INV-4/INV-5 actually hold

This repo already had these two failures. The `IRONBAR.md` ledger in the
parent superrepository records them as T19, T20, T21 and T25. This section
repeats them, because a future edit to this bar hits them again without this
knowledge:

- **A size on a `button`'s container alone does not size its child
  `label`.** The button packs the label at its natural width, aligned to
  the start, whatever the container's own `min-width` is. `justify: center`
  only aligns Pango lines against each other, not the layout inside the
  widget. INV-4's `.ws` fixes this: `.ws button` and `.ws button label`
  carry the same `min-width`. The label then fills the reserve, and its own
  `xalign: 0.5` centres the text. INV-5's pills want the opposite result,
  with the content packed at one edge and the slack at the other. They give
  `min-width` to the module's own node only. A `label` twin here would
  bring back the centring that this list warns against.
- **A bare class beats `#bar #end > *` in this engine.** T21 verified this
  on the live bar. It is the opposite of the W3C specificity model. Any
  per-pill override of padding or width MUST be a bare class rule. A rule
  qualified under `#bar #end` does not win.

---

## 3. Module specification

### 3.1 `start` — launcher, tags, focus

| # | Module | Interaction | Width | Rationale |
| - | --- | --- | --- | --- |
| 1 | `spark` | click → `rofi -show drun` | fixed | The most frequent click in the bar. It occupies the top-left magic corner (INV-2). |
| 2 | `ws-1…9`, `ws-ov` | click / scroll / right-click popup | fixed | The second most frequent click. The travel from the corner is the shortest. The X stays constant (INV-4). |
| 3 | `win` | none (scroll: brightness) | variable | The width changes most in the bar. `win` is last in `start`, thus it moves nothing (INV-1). `truncate.max_length` cuts the title at 32 characters. `win` is not a click target, apart from its own scroll. |

`kitty/repo-title.py` sets the title of a kitty window to
`"<repo> · <title>"`. That script is a kitty watcher, not a bar module.
`window_text()` in `mango.rs` splits the title on `" · "` when
`appid=="kitty"`. It then shows the repo as the dim first line, in place of
the appid. The split applies to the kitty appid only. Thus a Firefox tab
title with the same separator stays unchanged.

**Semantic reading order:** left to right, *identity → location → focus*.
This reads as "what can I launch / where am I / what am I doing".

The fallback bar has no monitor name, thus it cannot build the tag
variables. It renders `spark` and `win` with no tag block. See `build()`'s
own doc comment for the reason an unlisted output still gets a bar.

### 3.2 `center` — time

| # | Module | Interaction | Width | Rationale |
| - | --- | --- | --- | --- |
| 4 | `clock` | none (hover popup: world clocks) | fixed | The most frequent glance, and the weakest need for a click. The centre is the shortest eye movement from a screen-centre gaze. |
| 5 | `date` | click → calendar app; hover popup: month | fixed | The same semantic domain as the clock. |
| 6 | `pomo` | left: start/pause; right: mute; hover popup | fixed | The user clicks it a few times each day. The weak click target of this position is acceptable at that frequency. |

The three modules sit in the centre, not at the right edge. Glance-only
content in the centre keeps the eye travel short. A wider monitor makes this
more important. The position costs nothing, because none of the three needs
a click target of corner quality.

### 3.3 `end`, block 1 — tray

| # | Module | Interaction | Width | Rationale |
| - | --- | --- | --- | --- |
| 7 | `tray` | click → app window or menu | variable | It leads `end`, thus it takes its own growth alone (INV-1). It is the only member of `end` with a variable width. It is always visible, because the T29 follow-up sets `TRAY_DRAWER_ENABLED = false`. See below. |
| 7c | `keepass` | click → show/hide KeePassXC | fixed | Moved out of the drawer, because the lock state is worth a glance. |

**T28 — the tray drawer.** Ironbar's native `tray` module has no per-item
filter. It has `icon_size`, `direction` and `prefer_theme_icons` only, plus
the common options. Every item gets the same `.item` class, and no item gets
a name. Thus the tray itself cannot express "important against hidden".
KeePassXC is the one item on this machine that is worth a permanent glance.
It is rebuilt as its own pill, in place of an item kept out of the drawer.

`tray-click.sh {address}` also overrides `tray`'s own `on_click_left`. The
script jumps to an already-open window before it falls back to SNI
`Activate`. Thus a tray click can no longer hide a window that lives on
another tag. NordVPN is the only tray item with `ItemIsMenu: true`. A live
check with `busctl --user get-property` confirmed this. Its left-click menu
moves to right-click, and this spec accepts that one regression.

**T29 follow-up — drawer disabled by default.** The user prefers a tray icon
list that is always visible, with no toggle to hide it. The
`TRAY_DRAWER_ENABLED` constant in `genconfig.rs` defaults to `false`. With
that value it removes `tray`'s own `show_if: "#tray_open"` gate, and it
removes the `traytoggle` module. The drawer mechanism itself stays in place:
`tray_toggle_module()`, `tray-drawer.sh` and the `.traytoggle` CSS. It works
again as soon as the constant returns to `true`.

**T29 — Arch-Update leaves this block.** T28 moved it here for the same
reason `keepass` stays: a pending count is worth a glance. It now folds into
the docker row of the `devload` stack. See §3.4's own T29 entry for the
reason.

### 3.4 `end`, block 2 — resources

`cpu`, `memory`, `docker`, `battery`, `claudebar` and `archupdate` make one
ambient group for glances only. Each pill or stack has a hover popup. A
click starts an external tool: `btop`, `powermode.sh`, `powertop` or
`arch-update`. A right-click on docker opens a menu.

The group uses **proximity and common region**. These modules answer one
question: "is this machine healthy". The user reads them in one eye
movement, not in several separate stops. The order in the block reads the
load from general to specific: cpu → memory → docker (processes) → battery
(power) → claude → archupdate. `claude` is an external budget, with the
least relation to the machine itself. `archupdate` is routine maintenance,
not health.

**T28 — gauges replace the percentage.** `cpu` and `memory` drop their digit
completely. They show an icon and a horizontal `.gauge` box only. The gauge
carries the magnitude on the bar, and the popup keeps the exact number.
`battery` keeps its icon, digit and `%` text, and gains a `.gauge` next to
them. `docker` stays unchanged.

The gauge itself is a nested empty `box.gauge` widget. `ButtonWidget`'s
`label` and `widgets` fields are mutually exclusive, and `widgets` wins, as
a check against the vendored ironbar source confirmed. A CSS
`linear-gradient` hard stop fills the gauge, and a `pNN` class in 5% steps
selects that stop. The daemon pushes the `pNN` class on an **independent**
class slot (`@class/<module>#level`). netsec's own `eco` class already
established this `#`-suffix convention. Thus the level class never removes
the pill's own `warning` or `critical` state class.

The fill and the border both use `currentColor`. Thus the gauge of a warning
or critical pill turns `@urgent` by itself, and it needs no separate
per-state gauge rule. Ironbar's `progress` widget was rejected for this
task. Its `value` is a `ScriptInput`, which is a polling subprocess.
`mango-bard` exists to replace exactly that shape.

**T29 — two-row stacks, battery gets a terminal nub, the countdown comes
back.** `cpu` and `memory` merge into one `sysload` module, with two rows
one above the other. `claudebar`, `docker` and `archupdate` merge into one
`devload` module, with a claude row and then a docker and updates row.

A nested `custom` *module* cannot work here. Ironbar's `style add_class` and
`remove_class` find a target only in the bar's own top-level `start`,
`center` and `end` arrays (`bar.rs::add_modules`). A nested module's own
`ModuleRef` is discarded on the way in (`modules/custom/mod.rs::add_to`, in
the vendored source). Thus a nested `cpu` module answers "Module not found"
for every class push. Every gauge level and every warning colour then stops
working.

Each stack is instead one top-level module. Its `bar` is a
`box, orientation: "vertical"` that holds plain `button` and `box` rows. A
nested plain widget keeps its own `on_click_left` and `show_if` as a real
field (T23, confirmed live for popup hold and release). Thus a row loses
nothing of its own behavior. It loses only the ability to register its own
IPC-addressable class or popup.

That loss is the one real cost. Each stack has **one popup** that covers
every row (`popup_multi`). Every row's state class and level class lands on
the stack's single shared node, not on a node of its own. This forces a
`<slot>-<value>` naming convention: `cpu-warning`, `mem-warning`, `cl45`,
`ml45`, `claude-critical`, `dok-warning` and `au-pending`. A row's state
transition removes the old value and adds the new one. The convention stops
that transition from deleting a sibling row's current class off the shared
node.

`battery` gains a small `.gauge-cap` square right of its existing gauge.
`currentColor` fills that square, as it fills the gauge itself. The pair
then reads as a battery, a rectangle with a terminal nub, in place of a
plain rounded bar. `battery` drops its own `NN%` digit, as `cpu` and
`memory` did at T28, because the gauge now carries the level. `claudebar`'s
countdown comes back. T28 dropped that countdown to shrink a pill that stood
alone, and the claude row now shares its space with nothing else that needs
the width.

### 3.5 `end`, block 3 — tools

T31 removed the T23 tools drawer. The permanent "…" trigger cost the same
scan attention that Hick's Law charges for the icons. It also added a hover
step to reach them. The user prefers tools that are always visible.

`colorpicker`, `darkmode` and `snip` are standalone pills again, in the T23
reading order. Each one is a `custom` module with a static tooltip, and its
T20 and T23 clicks stay unchanged. **`inhibit`** follows them, also
unchanged, because keep-awake is a state the user glances at. It renders at
constant width in both states, because the `nf-md-coffee` and
`nf-md-coffee_outline` glyphs have the same size.

### 3.6 `end`, block 4 — audio

`music`, `volume` and `mic` make one domain by proximity and common region.
They show what plays, and what the machine does with sound.

- **`music`** — `music.rs` reads MPRIS with `playerctl --follow`. The text
  truncates at 24 characters, thus it cannot grow past its slot. `music`
  sits in the middle of the block, not at a growth end. INV-1 therefore
  requires this fixed cap in place of an open-ended width.
  - **T26 deviation:** the CSS carried a `min-width: 220px` reserve against
    that cap. A live measurement showed that real track titles never come
    near 30 characters. The reserve then stayed empty behind a short title
    or behind nothing, and it was the largest gap on the bar. The reserve
    is removed, so `volume` and `mic` now shift when a track starts or
    stops. That shift is a discrete event caused by the user. It is not
    the per-tick reflow that INV-1 guards against, thus this spec accepts
    it.
  - **T28: native `music` module replaced with a `custom` one.** The native
    module renders through GTK4's `set_label_escaped`, as a check against
    the vendored source `modules/music/mod.rs` confirmed. That function
    renders real text only, never markup. Thus the native module could not
    carry the two-line markup of a dim app name and a plain title that the
    window pill (§3.1) uses. `music.rs` now feeds `music_text` directly,
    in that same two-line shape. `show_if: "#music_on"` replaces T26's
    handling of emptiness by truncation alone. The pill now disappears
    completely when nothing is loaded, or when a player is paused with no
    track, in place of a reserve for an empty string. T26's own reasoning
    about a discrete event caused by the user applies unchanged.
- **`volume`** — an icon only on the bar. T9 moved the percentage to the
  popup. A left click opens a 5s mixer. A right click opens the full
  mixer. A scroll adjusts the volume.
- **`mic`** — an icon for the mute state. It renders empty when the
  microphone is not muted. Before T29 it shared `.volume`'s
  `min-width: 20px` reserve. That reserve held a full icon's width even
  while the module was empty, and it was the real cause of a visible gap
  between `volume` and `wifi`. `mic` now has no reserve of its own, so the
  common unmuted case adds no width. A mute moves `wifi` by one icon
  width, which is the same trade this spec already accepts for `music`.

### 3.7 `end`, block 5 — connectivity

`net-spinner`, `wifi`, `eth`, `netsec`, `hotspot`, `remote` and `bluetooth`
make one block by proximity and common region. "How am I connected, and is
it safe" is one question with seven possible answers. It is not seven
separate questions. This block is the largest single consolidation in the
layout. Thus the internal order is part of the spec. It runs from general to
specific:

1. `net-spinner` — a busy indicator. It replaces `wifi` in place while a
   link changes state. Its `show_if: "#net_busy"` is the logical inverse of
   `wifi`'s own `show_if`.
2. `wifi` — link state and signal.
3. `eth` — wired link state.
4. `netsec` — the security verdict *for* the link that `wifi` or `eth` just
   described. It reports a DNS leak, a route conflict, a captive portal or
   an open network. It comes immediately after the links it grades.
5. `hotspot` — a *mode* of the same wifi radio that `wifi` covers.
   `show_if` hides it until it is active, thus it costs nothing when
   unused.
6. `remote` — inbound reachability through wayvnc or KDE Connect. It
   answers the same "who can reach this machine" question as the security
   pill. It sits outward from `netsec`, not inside it, because it is a
   different protocol family with its own click target.
7. `bluetooth` — the nearest remaining connectivity radio. It is last in
   the block, because it is the one native module with a click-to-toggle
   popup that must sit next to `power`'s own corner treatment. T9 found
   that its popup does not obey `popup_autohide`. An unconditional
   `hover_exit` and `hide_popup` close it instead. See
   `bluetooth_module()`'s own doc comment.

### 3.8 `end`, block 6 — session

| # | Module | Interaction | Width | Rationale |
| - | --- | --- | --- | --- |
| 18 | `power` | click → **menu** (`powermenu.sh`) | fixed | The top-right magic corner. It opens a menu only, per INV-3. |

---

## 4. Permitted variant

This repo does not take this variant now. `volume` and `power` MAY be
swapped, so that `volume` occupies the top-right corner. Take that swap only
if the user opens the power menu rarely, and adjusts the volume by a scroll
on the bar. A scroll target in a magic corner is the fastest volume control:
move, scroll, done, with no click and no precision. The swap also
strengthens INV-3, because a scroll adjustment is not destructive.

This spec specifies no other reordering. A move of the clock off `center` is
**out of scope**.

### 4.1 `HEADLESS-*` bars — a deliberate exception

This document describes a bar with a user in front of its own monitor. The
bar that ironbar builds for a `HEADLESS-*` output is different. That output
is wayvnc's capture surface (see `system/remote/README.md`). Nobody looks at
that output's own tags. The user looks at the physical tag pulled onto it.
The `HEADLESS` branch in `genconfig.rs::build()` exempts this bar from §3 by
design:

- The bar has no `center` and no `end`. The generator omits them, and does
  not build them empty. §3.2-3.8 do not apply.
- `start` is a remote-control strip, not launcher→tags→focus. It holds one
  private pill. It then holds a name label and nine pull pills for each
  physical monitor (`remote_pills()`). INV-4's "constant X" still applies,
  because the member count per monitor is fixed and `ws.empty` dims a pill
  in place of hiding it. INV-2's magic corner does not apply, because this
  bar has no `spark` and no `power` to bleed into it.
- The remote pills drop the `show_if` gate that INV-1 wants elsewhere. See
  `remote_pills()`'s own doc comment. A gate on *that monitor's* overview
  state would reflow this bar for a screen the viewer does not look at.
  That reflow is worse than the small, bounded width change that INV-1
  prevents elsewhere.

A future pass should not change this bar to match §3. It is a different kind
of bar by design. It is not an incomplete bar.

---

## 5. Reference implementation (ironbar / `mango-bard`)

The generator makes the config. Nobody writes it by hand. See
`ironbar/bard/src/genconfig.rs`. This is the shape, with the module bodies
omitted. Read the file itself for the full JSON that each builder returns:

```rust
// genconfig.rs::build(), per real monitor bar
"start":  start_modules(&bar_name),   // spark, workspace_pills(), win
"center": time_modules(&bar_name),    // clock, date, pomo
"end":    end_modules(&bar_name),     // tray, keepass,
                                       // resource_modules(), tools_modules(),
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

// T29 follow-up: TRAY_DRAWER_ENABLED = false by default — tray_toggle_module()
// is only pushed, and tray_module()'s own show_if only set, when true.
fn end_modules(bar_name: &str) -> Vec<Value> {
    let mut end = vec![tray_module()]; // INV-1: leads end
    if TRAY_DRAWER_ENABLED {
        end.push(tray_toggle_module());
    }
    end.push(keepass_module(bar_name));     // promoted out of the tray drawer
    end.extend(resource_modules(bar_name)); // sysload, battery, devload
    end.extend(tools_modules(bar_name));    // darkmode, inhibit (T31)
    end.extend(audio_modules(bar_name));    // music, volume, mic
    end.extend(net_modules(bar_name));      // net-spinner, wifi, eth, netsec
    end.push(hotspot_module(bar_name));
    end.push(remote_module(bar_name));
    end.push(bluetooth_module(bar_name));
    end.push(power_module());
    end
}

// T29: sysload (cpu+memory) and devload (claude, docker+archupdate) are
// each one top-level module — a vertical `box` of plain button/box rows,
// not nested `custom` modules (a nested module's own class updates never
// reach ironbar's style IPC — see sysload_module()'s own doc comment).
fn sysload_module(bar_name: &str) -> Value { /* box, orientation: vertical,
    widgets: [row-cpu button, row-mem button] */ }
fn devload_module(bar_name: &str) -> Value { /* box, orientation: vertical,
    widgets: [row-claude button, row-svc box[cell-docker, cell-au]] */ }
```

The tools block (T31 — drawer removed, see §3.5):

```rust
fn tools_modules(bar_name: &str) -> Vec<Value> {
    vec![
        colorpicker_module(),
        darkmode_module(),
        snip_module(),
        inhibit_module(bar_name),
    ]
}
```

The CSS is in `matugen/templates/ironbar/style.css`. The selectors are
classes that `genconfig.rs` sets. They are not waybar-style `#custom-*` ids:

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
   distinction from INV-4's `.ws`). volume is icon-only (T9) — min-width
   alone, no tnum, per §2's own distinction. */
.sysload, .devload, .battery, .clock, .date, .pomo,
.sysload label, .devload label, .battery label,
.clock label, .date label, .pomo label {
  font-feature-settings: "tnum" 1;
}
.sysload { min-width: 40px; }       /* T29: icon + .gauge per row, no digit */
.battery { min-width: 56px; }       /* T29: gauge + gauge-cap, digit dropped */
.devload { min-width: 131px; }      /* T29: claude row's countdown is back */
.clock { min-width: 50px; }
.date  { min-width: 84px; }
.pomo  { min-width: 52px; }
.volume { min-width: 20px; }
.keepass { min-width: 20px; }       /* T28 — icon-only, same as volume */
/* T29: `.mic` reserve dropped — it was the real cause of the volume-wifi
   gap (§3.6's own T29 entry). T26: music has no reserve (dropped, see
   §3.6's own T26 deviation) — the .music selector above shares the
   icon-font stack rule only. */

/* T28 — INV-5's gauge variant: the fill's own width IS the value, so
   there is nothing to reserve beyond the box's own fixed size (§3.4).
   T29: cpu/memory share `sysload`'s one node, so their level class needs
   its own prefix per row (`clNN`/`mlNN`, scoped `.clNN .row-cpu .gauge`)
   — a bare `pNN` on both would let one row's transition delete the
   other's still-current class off that shared node. `battery` keeps the
   bare `pNN` — still its own module, no collision possible. */
.gauge { min-width: 24px; min-height: 10px; border: 1px solid currentColor; }
.p50 .gauge { background: linear-gradient(to right, currentColor 50%, transparent 50%); }
.cl50 .row-cpu .gauge { background: linear-gradient(to right, currentColor 50%, transparent 50%); }
.ml50 .row-mem .gauge { background: linear-gradient(to right, currentColor 50%, transparent 50%); }
/* ...one rule per 5%-step class, 0 through 100, for each of p/cl/ml
   (style.css has all 63). */
```

---

## 6. Verification checklist

Verify a width claim or a position claim by **pixel-column analysis of a
`grim` crop, never by eye**. T19 measured "pixel-perfect" by eye and was
7.5px out. Always pass `grim -o <output>`. A bare `grim` captures every
monitor, and everything else on screen.

- [ ] Move the pointer to the top-left at full speed. `spark` activates on
      click, with no dead pixel.
- [ ] Move the pointer to the top-right at full speed. The `power` **menu**
      opens. No action fires, and no state changes.
- [ ] Start a tray application, then quit it. No module but the tray
      changes position.
- [ ] Focus a window with a 100-character title. The tags and `spark` do
      not move.
- [ ] Move `sysload`, `battery` and `devload` each from the realistic
      maximum to the rare overflow: either gauge row at 99%→100%, docker
      9→99, 100%→100%+eco leaf, with and without the stale-cache marker.
      `tray`, the leftmost module in `end`, does not shift by a single
      pixel. `center` is the wrong subject for these three modules. They
      moved from `start` to `end` at T23, and `end` is right-anchored, so
      growth pushes `end`'s own leftmost member, `tray` (T25). T29's own
      reserve numbers for `.sysload`, `.battery` and `.devload` are
      estimates. Nobody has measured them live. This check corrects them.
- [ ] Move `pomo` from idle to running to idle. The `center` width does not
      change.
- [ ] All 9 tags stay visible with every tag empty. Toggle `ws-ov` on and
      off. `wifi` does not shift.
- [ ] Move from tag 9 to overview to tag 1. No button changes width.
- [ ] Click `darkmode`. The colour scheme flips, and the pill's icon
      follows (T31: drawer removed, darkmode standalone).
- [ ] Toggle `hotspot` from active to inactive. The neighbouring `remote`
      and `bluetooth` do not shift.
- [ ] Play a long MPRIS track title. `music` truncates at 30 chars, and
      `volume` does not move.

---

## Sources

The layout comes from the following sources. No controlled A/B test exists
for status-bar module ordering. The outer anchors use Fitts's law and the
research on corner and edge targets. The interior blocks that §3 introduces
use Gestalt grouping and Hick's Law.

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
- The `IRONBAR.md` ledger in the parent superrepository, T19–T22 — the two
  GTK4 traps (§2) and the T22 decision to empty `center`. This spec's §1
  and §3.1 reverse that decision by design. The ledger's T23 entry gives
  the reason the reversal is safe.

This document holds two embedded assets: the ASCII bar diagram (§1) and the
Rust and CSS reference implementation excerpts (§5). The author writes both
inline in this file. No external generator, no build step and no separate
source artifact exists for them.
