# Work inbox

Press `w` in the remote TUI to open the owner work inbox. The view reads the
current machine inventory and scans sessions across machines. It uses the
existing session pages, agent checkpoints, and approval index.

Each page reads at most 16 sessions or 16 machines. Press `n` for the next page.
Press `r` to start a new scan. Empty attention results apply only to the scanned
page. Pages are not a transaction snapshot. New sessions before a cursor require
a new scan. A scan can include ended sessions that remain in the owner inventory.

The view shows native process lifetime and typed agent activity separately. A
failed agent turn needs attention even when its process is still running. PTY
text does not establish task outcome. Missing activity, incomplete approval
coverage, unavailable machines, and recording warnings remain explicit.
The snapshot age increases after a read. Refresh the view before you act.

Use these keys:

- `Enter`: Open the selected session in the normal session view. Use `v` for its
  live view or `Enter` again for protected history.
- `a`: Open the existing owner approval inbox. Approval choices retain their
  existing authorization. Work entries do not grant control or approval rights.
- `!`: Switch between attention entries and all scanned sessions.
- `x`: Acknowledge an exact successful turn completion for this TUI instance.
- `Esc`: Return to the previous view.

Completion acknowledgments survive inbox navigation and observation reconnects
within the same gateway TUI instance. Quitting the TUI clears them. Each key
contains owner, machine, session, and turn IDs. Acknowledgments cannot hide
pending approvals, failures, incomplete coverage, or recording warnings. The
local set holds at most 4096 keys and refuses additional acknowledgments when
full. It stores no task text and sends no acknowledgment to the native agent.

Each page owns a separate observation connection. Native checkpoint reads run
one at a time. A checkpoint has a two-second budget. A page scan has a ten-second
budget after inventory and approval reads. Closing the view cancels owned reads
and closes their connection. It never ends a native session. Unavailable reads
remain unknown. Refresh retries observation only.

The gateway continues to enforce owner scope, revocation, and retention. The
view does not create a second index or copy protected recording content. An old
gateway without session pagination reports an unsupported capability.
