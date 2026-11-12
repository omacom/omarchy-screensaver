# D-Bus observation

The screensaver publishes read-only animation state on the user session bus so another presentation target can follow its effects with its own layout. No observer is required for normal operation. Bus failures leave normal rendering operational.

- Bus name: `org.omarchy.Screensaver` (owned only while the native overlay runs)
- Object: `/org/omarchy/Screensaver`
- Interface: `org.omarchy.Screensaver1`
- Method: `GetState() -> Snapshot`
- Signal: `StateChanged(Snapshot)`

`Snapshot` is a D-Bus struct with fields in this order:

| Field | Type | Meaning |
| --- | --- | --- |
| revision | `t` | Monotonically increasing within this bus owner; starts at zero before the first effect |
| active | `b` | Whether a reproducible effect description is available |
| engine | `s` | Initialization compatibility identifier, currently `ttfx-0.5.1` |
| effect | `s` | Selected effect name |
| input | `s` | Resolved input text, including its existing formatting |
| rng_state | `at` | Four xoshiro state words captured after selection and before `EngineCtx`/layout initialization |

Geometry and anchoring are deliberately local to each observer. TTY preparation/cleanup options are not part of this interface. The description reproduces the chosen initialization with the defaults of the identified engine version. It does not promise matching trajectories or completion times for different grids. There is no seek operation or per-frame timing stream.

Subscribe to owner changes and `StateChanged` before calling `GetState` on the current unique owner. Accept only that owner's signals; discard revisions already applied. Reset the revision when the owner changes. An owner disappearing terminates observation. There is no D-Bus activation file: a read must not launch the screensaver.

Updates may coalesce; `GetState` returns the latest snapshot. Input over 64 KiB produces an inactive snapshot without truncating the screensaver's actual input. Readers cannot start, stop, select, or control effects through this interface.

Initial construction, an actual geometry rebuild, and completion followed by a new effect produce new revisions. Ordinary frames do not. Late observers initialize the current effect from its beginning and adopt subsequent transitions. This is a current-state stream, not a durable history.

Observers must support the advertised engine identifier before using its RNG state. Restore that state before constructing the selected effect with the resolved input and the observer's geometry. Reusing the original CLI seed would omit random draws consumed while selecting the effect.
