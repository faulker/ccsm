# CCSM UI/UX + Keybinding Refactor Plan

Findings and remediation plan for the goal in `PLAN.md`: *"a clean, usable UI/UX
that shows the relevant shortcut keys even on a small screen."*

All measurements below were taken against the real code (file:line cited) and,
where noted, verified by rendering through ratatui's layout solver.

---

## 1. Findings

### 1.1 The status bar is the core problem, and it fails silently

`render_status_bar` (`src/ui/info_bar.rs:199`) builds a flat, unranked list of
14–15 hints and hands them to `render_hint_row` (`src/ui/info_bar.rs:149`), which
lays them out as fixed `Constraint::Length` chunks. When the terminal is narrower
than the sum of those lengths, ratatui hands the trailing chunks zero width. They
do not truncate, wrap, or show an overflow marker. **They just disappear, with no
indication that anything is missing.**

Measured hint widths (sum of key span + label span, plus the minimum 1-column gap
and the 8-column version label at `src/ui/info_bar.rs:201`):

| Bar variant | Hints | Min terminal width to show all |
|---|---|---|
| Sessions tab | 14 | **158 cols** |
| Sessions tab, live session selected | 15 | **173 cols** |
| Jobs tab | 14 | **150 cols** |

What actually survives at common widths:

| Width | Sessions | Jobs |
|---|---|---|
| 60 cols | 5 of 14 | 5 of 14 |
| 80 cols | 7 of 14 | 7 of 14 |
| 100 cols | 8 of 14 | 9 of 14 |
| 120 cols | 10 of 14 | 11 of 14 |

**The priority order is exactly backwards.** `q quit` and `? help` are pushed last
(`src/ui/info_bar.rs:398-405`), so they are the *first* two hints to vanish. On a
standard 80×24 terminal the user cannot see how to get help or how to quit. Every
hint that would have told them where the other hints went is gone.

Secondary status-bar issues:

- The `v{version}` label (`src/ui/info_bar.rs:201`) reserves 8 columns on every
  screen, at every size, permanently. It is the least operationally useful text on
  screen and it is the only element guaranteed never to be dropped.
- The Shift-preview mechanic (`shift_active`, `src/ui/info_bar.rs:250-259`,
  `src/keys.rs:340-356`) relabels hints while Shift is held. It depends on the
  enhanced keyboard protocol reporting bare modifier press/release. In terminals
  that do not, the labels never change — but the `D`/`W`/`l`/`?` hints are styled
  with `shift_key_style` *unconditionally* (`src/ui/info_bar.rs:373-378`,
  `385-386`, `403`), so they render permanently "lit" as if Shift were down. The
  bar contradicts itself.
- Three of the widest hints are variants of a single action: `D new dangerous`
  (15 cols), `W new worktree` (14), `n new live` (10). 39 of the 137 hint columns
  in the Sessions bar are spent on one concept.

### 1.2 Keybindings: real inconsistencies

| # | Issue | Evidence |
|---|---|---|
| 1 | **`Esc` quits the whole app** in Sessions/Normal, but means "go back" in the Jobs tab and "cancel" in every modal. An `Esc` pressed once too many while backing out of a popup exits the app. | `src/keys.rs:477` vs `src/ui/jobs_tab.rs:50` vs all modal handlers |
| 2 | **`f` is favorite in Sessions, mark-done in Jobs.** One is a harmless pin, the other resolves a job. `f` is also a weak mnemonic for "done". | `src/keys.rs:525` vs `src/ui/jobs_tab.rs:87` |
| 3 | **`x` stops without confirmation in Sessions but opens a confirm modal in Jobs.** Same key, same verb, asymmetric safety. | `src/keys.rs:545` vs `src/ui/jobs_tab.rs:85` |
| 4 | **`w` opens the Jobs tab, `Shift+W` creates a worktree session.** Same letter, unrelated meanings, and `w` is fully redundant with `Tab`. | `src/keys.rs:533` vs `src/keys.rs:590` vs `src/keys.rs:488` |
| 5 | **`q` quits from the Jobs tab but `Esc` does not**; in Sessions both do. The relationship inverts between tabs. | `src/ui/jobs_tab.rs:47,50` vs `src/keys.rs:477` |
| 6 | **Confirmation keys differ per modal.** Job confirm: `y`/`n`/Enter/Esc. Update prompt: `y`/`n`/Esc. Duplicate popup: `o`/`r`/Enter/Esc, with no `y`/`n` at all. | `src/ui/jobs_tab.rs:203-204`, `src/keys.rs:370-380`, `src/keys.rs:271-296` |
| 7 | **`c` resumes a job in Jobs but is unbound in Sessions**, while `Ctrl+C` quits in both. | `src/ui/jobs_tab.rs:84` |
| 8 | **`d`/`Shift+D` mean opposite things.** Jobs `d` = delete (destructive, confirmed). Sessions `Shift+D` = launch with `--dangerously-skip-permissions` (destructive, *unconfirmed*, one Shift away). | `src/ui/jobs_tab.rs:86` vs `src/keys.rs:587` |
| 9 | **`l` has three meanings**: live filter (Sessions), watcher log via `L` (Jobs), next help page (help overlay). | `src/keys.rs:539`, `src/ui/jobs_tab.rs:90`, `src/keys.rs:244` |
| 10 | **View mode, hide-empty and group-chains have no keybinding at all.** `cycle_view_forward`/`cycle_view_backward` (`src/app/display.rs:15,44`) are reachable only from inside the config popup. | `src/config_popup.rs:161-176` |

### 1.3 Discoverability gaps

Bound in code, absent from the status bar:

- `m` — create/edit a job from the current selection (`src/keys.rs:536`). In the
  help popup only (`src/ui/modals.rs:294`).
- `Tab` / `BackTab` — tab switching (`src/keys.rs:488-493`). Only in the tab strip.
- `Shift+Enter` — open historical session directly (`src/keys.rs:593`).
- `←`/`→` — collapse/expand and jump-to-parent in tree view (`src/keys.rs:666,693`).
- `Ctrl+C` — quit (`src/keys.rs:480`).
- `o` — config, from the Jobs tab (`src/ui/jobs_tab.rs:56`).
- `i` — type a value by hand in the job form and config popup
  (`src/ui/jobs_tab.rs:181,186`, `src/config_popup.rs:222`).

### 1.4 Modals clip on small terminals, with no way to scroll

Verified by running the real `centered_rect` (`src/ui/util.rs:148`) through
ratatui's solver:

| Terminal | Help popup | Config popup | Job form | Duplicate |
|---|---|---|---|---|
| 60×20 | 42×16 | 36×16 | 38×16 | 26×4 |
| 80×24 | 56×20 | 48×20 | 52×20 | 36×4 |
| 100×30 | 70×24 | 60×24 | 64×26 | 44×6 |

- **The help popup has no scrolling.** At 80×24 it gets 20 rows: minus 2 border,
  2 tab header, 1 footer leaves 15 content rows for a Sessions page of ~24 lines
  (`src/ui/modals.rs:282-309`). Roughly a third of the help text is unreachable —
  including, on the Sessions page, the entire "Rename mode" section. The help
  popup is the fallback for everything the status bar drops, so this compounds
  finding 1.1 rather than mitigating it.
- **The config popup has no scrolling** either, and is hard-capped at 28 rows
  (`src/config_popup.rs:278-279`) against ~28 lines of content. Below a 35-row
  terminal, the bottom settings and the About block are unreachable.
- The duplicate popup gets 4 rows at 80×24 for 5 lines of content
  (`src/ui/modals.rs:69`).
- The naming popup's `centered_rect(46, 3, …)` resolves to **zero height** below a
  ~34-row terminal, but is rescued by an explicit clamp at
  `src/ui/modals.rs:30-34`. Not a live bug — but it is the only popup with that
  guard, which is why it is the only one that survives.

### 1.5 Other UI observations

- The Sessions list title (`build_title_spans`, `src/ui/info_bar.rs:120-140`)
  accumulates up to four state badges (`[showing empty]`, `[ungrouped]`,
  `(path)`, `[live only]`) plus activity counts, inside a pane that is only 30%
  of the width — 24 columns on an 80-column terminal. It overflows long before
  the status bar does.
- The Jobs tab and Sessions tab both use the 30/70 split, but the Sessions tab
  adds a 3-row info header (`src/ui/mod.rs:186`) and the Jobs tab a variable-height
  banner (`src/ui/jobs_tab.rs:307`). Vertical rhythm differs between siblings.

---

## 2. Plan

Ordered by value-per-risk. Phases 1 and 2 deliver the stated goal on their own.

### Phase 1 — Make the status bar responsive and honest

The rule: **never silently drop a hint.** Give each hint a priority, fit as many
as the width allows in priority order, and show an explicit overflow marker
pointing at `?` when anything was cut.

1. Introduce a `Hint { key, label, priority }` model in `src/ui/info_bar.rs`,
   replacing the ad-hoc `Vec<Line>`.
   - `P0` — never dropped: `? help`, `q quit`.
   - `P1` — primary interaction: navigate, `Enter` (open/attach), `n` new.
   - `P2` — frequent: `/` search, `r` rename / `e` edit, `x` stop, `Tab` switch.
   - `P3` — everything else; lives in the help overlay.
2. Rewrite `render_hint_row` to measure first: fit P0 hints, then fill with P1,
   P2, P3 while the width allows, and render `…` before the P0 tail when
   anything was dropped.
3. Shorten labels (`new live` → `new`, `new dangerous` → `danger`,
   `auto-resume` → `auto`, `watcher log` → `log`).
4. Move the version label out of the status bar into the config popup's About
   block, reclaiming 8 columns at every size.
5. Fix the Shift-preview inconsistency: style `D`/`W`/`l`/`?` with the same
   conditional `shift_active` treatment as the other hints, so the bar stops
   claiming Shift is held when it is not.

Target: the Sessions bar fits in **60 columns** at P0+P1+P2, and degrades to
`? help  q quit  …` rather than to nothing.

### Phase 2 — Make the help overlay a real fallback

Everything Phase 1 drops has to be findable, so the help overlay must be complete
and reachable.

1. Add scrolling to the help popup (`j`/`k`, `↑`/`↓`, `PgUp`/`PgDn`) with a
   position indicator in the footer, and a new `help_scroll` field on `App`.
2. Add the same to the config popup.
3. Add a `centered_rect_min(pct_x, pct_y, min_w, min_h, area)` helper to
   `src/ui/util.rs` and route every popup through it, so no modal can resolve
   below its content minimum. Remove the one-off clamp at `src/ui/modals.rs:30-34`.
4. Fill the documented gaps from §1.3 into the help pages: `Tab`, `Shift+Enter`,
   `←`/`→`, `Ctrl+C`, `i`, and `o` under the Jobs page.

### Phase 3 — Refine the keymap

Principle: **the same verb gets the same key in both tabs; `Esc` never destroys.**

| Change | From | To | Rationale |
|---|---|---|---|
| `Esc` in Sessions/Normal | quits the app | clears filter if set, else no-op | Finding 1, the single biggest safety fix. `q` and `Ctrl+C` still quit. |
| Favorite | `f` | `Space` | Frees `f`, and matches Jobs where `Space` is already the toggle key. Favorite *is* a toggle. |
| Mark job done | `f` | `f` (unchanged) | Once favorite moves, the collision is gone. |
| Open Jobs tab | `w` | dropped | Redundant with `Tab`; frees the `w`/`Shift+W` collision. |
| Stop live session `x` | no confirm | confirm modal | Matches Jobs `x`. |
| Duplicate popup | `o`/`r`/Enter/Esc | adds `y`/`n` aliases | One confirm vocabulary across all modals. |
| View mode cycle | config popup only | `v` in Sessions | `cycle_view_forward` currently has no key at all. |

Deliberately **not** changed: `j`/`k`, `Enter`, `n`, `/`, `o`, `?`, `Tab`,
`Ctrl+C`, and the Jobs verbs `e`/`p`/`c`/`x`/`d`/`s` — these are either already
consistent or too load-bearing on muscle memory to move for marginal gain.

### Phase 4 — Trim the Sessions list title

Replace the four spelled-out badges in `build_title_spans` with compact glyphs
(e.g. `∅` showing-empty, `⚡` live-only) so the 30%-width pane stops overflowing,
and keep the spelled-out state in the config popup where there is room.

### Phase 5 — Tests

Per the repo's existing pattern (`TestBackend` frame rendering, see
`src/ui/jobs_tab.rs` and `src/config_popup.rs`):

- Render the status bar at 60, 80, 100 and 160 columns and assert `? help` and
  `q quit` are present in **every** case, and that the overflow marker appears
  exactly when a hint was dropped.
- Assert the priority ordering drops P3 before P2 before P1.
- Render the help and config popups at 60×20 and assert the last content line is
  reachable by scrolling.
- Keymap tests: `Esc` in Sessions/Normal does not set `should_quit`; `Space`
  toggles favorite; `x` on a live session opens the confirm modal.

---

## 3. Open decisions

These change the shape of the work and are not mine to assume:

1. **The three new-session variant keys** (`Shift+N` direct, `Shift+D` dangerous,
   `Shift+W` worktree) are 39 of the Sessions bar's 137 hint columns. Collapsing
   them into mode toggles inside the naming popup would be the single largest
   win — but `CLAUDE.md` records the opposite as a deliberate decision ("New-session
   launch modes are chosen by the key, not inside the popup"). Overturn it, or
   keep the keys and simply demote them to `P3`?
2. **How far to take the rebind.** Phase 3 as written breaks muscle memory for
   `f` (favorite → Space), `w` (dropped) and `Esc` (no longer quits).
3. **Whether the version label leaves the status bar.**
