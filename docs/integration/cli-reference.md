# Command reference

This file is generated from the executable Clap definitions. Do not edit it by hand.
See [interface checks](interface-checks.md) for the update procedure and baseline policy.

## `qgenie-remote`

```text
Local terminal sessions with private same-user IPC, durable start keys and replay; remote sessions, scoped sign-in, readable output and protected recorded history.

Usage: qgenie-remote [OPTIONS] [COMMAND]

Commands:
  host       Enroll, connect, pause and diagnose the independently owned remote host
  remote     Sign in and open remote sessions, or use scriptable owner operations
  tui        Start the interactive machine/session interface
  up         Start the independent local session daemon, if it is not running
  sessiond   Run the local session owner in the foreground
  status     Show local machine and daemon status
  machines   List available machines (the local machine in this preview)
  list       List running and historical local sessions
  run        Start a command in a PTY; keep its printed key to reconcile uncertainty
  attach     Open a session in observe mode; Ctrl-] c requests input control
  reconcile  Look up the original start key without sending a new launch
  send       Send one explicit input, release control, and never retry it automatically
  interrupt  Interrupt the foreground turn without terminating the shell
  terminate  Terminate a session explicitly
  stop       Stop the daemon; refuse while sessions remain unless termination is explicit
  doctor     Check local IPC, roots, capabilities, and preview limits
  logs       Read daemon diagnostics. PTY content is not written to this log
  help       Print this message or the help of the given subcommand(s)

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

## `qgenie-remote host`

```text
Enroll, connect, pause and diagnose the independently owned remote host

Usage: qgenie-remote host [OPTIONS] <COMMAND>

Commands:
  setup     Write an initial private configuration; enrollment and startup remain explicit
  up        Resume the host using user services, or independent background processes explicitly
  service   Install or manage separate user services. Connector operations preserve sessiond
  enroll    Enroll with an injected current remote user credential; no user token is stored
  login     Scoped PKCE/device sign-in and enrollment; refresh tokens are discarded
  connect   Run one machine-only outbound connector in the foreground (native owner is separate)
  down      Pause only this connector. Running sessions and their recording remain owned by sessiond
  status    Show current local enrollment, connector lock and native IPC state
  doctor    Verify native IPC, allowed roots and an actual TLS machine-authentication exchange
  unenroll  Revoke this enrollment using an owner credential; preserves native sessions
  help      Print this message or the help of the given subcommand(s)

Options:
      --config <CONFIG>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host setup`

```text
Write an initial private configuration; enrollment and startup remain explicit

Usage: qgenie-remote host setup [OPTIONS] --gateway <GATEWAY> --allow-root <ROOTS>

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --allow-root <ROOTS>
          

      --root <ROOT>
          

      --name <NAME>
          

      --ca-bundle <CA_BUNDLE>
          

      --allow-private-file
          Explicit fallback when the native credential store is unavailable

  -h, --help
          Print help
```

## `qgenie-remote host up`

```text
Resume the host using user services, or independent background processes explicitly

Usage: qgenie-remote host up [OPTIONS]

Options:
      --no-service
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host service`

```text
Install or manage separate user services. Connector operations preserve sessiond

Usage: qgenie-remote host service [OPTIONS] <COMMAND>

Commands:
  install         Generate private user units/plists and pin this executable. Does not start/restart jobs
  up              Idempotently start native ownership and resume/restart only the connector
  pause           Stop only the connector job, preserving native processes and capture
  upgrade-native  Upgrade sessiond only after its atomic administrative shutdown confirms it is idle
  status          Ask the OS user-service manager for both jobs' actual state
  help            Print this message or the help of the given subcommand(s)

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host service install`

```text
Generate private user units/plists and pin this executable. Does not start/restart jobs

Usage: qgenie-remote host service install [OPTIONS]

Options:
      --directory <DIRECTORY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host service up`

```text
Idempotently start native ownership and resume/restart only the connector

Usage: qgenie-remote host service up [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host service pause`

```text
Stop only the connector job, preserving native processes and capture

Usage: qgenie-remote host service pause [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host service upgrade-native`

```text
Upgrade sessiond only after its atomic administrative shutdown confirms it is idle

Usage: qgenie-remote host service upgrade-native [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --yes
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host service status`

```text
Ask the OS user-service manager for both jobs' actual state

Usage: qgenie-remote host service status [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host enroll`

```text
Enroll with an injected current remote user credential; no user token is stored

Usage: qgenie-remote host enroll [OPTIONS] --token-file <TOKEN_FILE>

Options:
      --state-dir <STATE_DIR>
          

      --token-file <TOKEN_FILE>
          

      --acknowledge-recording
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host login`

```text
Scoped PKCE/device sign-in and enrollment; refresh tokens are discarded

Usage: qgenie-remote host login [OPTIONS] --oauth-config <OAUTH_CONFIG>

Options:
      --oauth-config <OAUTH_CONFIG>
          

      --state-dir <STATE_DIR>
          

      --device
          

      --root <ROOT>
          

      --acknowledge-recording
          

  -h, --help
          Print help
```

## `qgenie-remote host connect`

```text
Run one machine-only outbound connector in the foreground (native owner is separate)

Usage: qgenie-remote host connect [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host down`

```text
Pause only this connector. Running sessions and their recording remain owned by sessiond

Usage: qgenie-remote host down [OPTIONS]

Options:
      --keep-sessions
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --terminate-sessions
          

  -h, --help
          Print help
```

## `qgenie-remote host status`

```text
Show current local enrollment, connector lock and native IPC state

Usage: qgenie-remote host status [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host doctor`

```text
Verify native IPC, allowed roots and an actual TLS machine-authentication exchange

Usage: qgenie-remote host doctor [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote host unenroll`

```text
Revoke this enrollment using an owner credential; preserves native sessions

Usage: qgenie-remote host unenroll [OPTIONS] --token-file <TOKEN_FILE>

Options:
      --state-dir <STATE_DIR>
          

      --token-file <TOKEN_FILE>
          

      --root <ROOT>
          

      --yes
          

  -h, --help
          Print help
```

## `qgenie-remote remote`

```text
Sign in and open remote sessions, or use scriptable owner operations

Usage: qgenie-remote remote [OPTIONS] [COMMAND]

Commands:
  setup                 Save connection settings, then use `remote` to sign in and open the TUI
  profile               Manage local launch profiles without saving credentials
  wait-output           Wait for a literal in fresh PTY output after an explicit event cursor
  wait-many             Observe up to 16 exact targets. A resolved target may have failed or need approval
  read                  Read a bounded plain-text checkpoint without controlling or scrolling the session
  events                Emit bounded JSON lines. Checkpoint records explicitly mark unavailable event history
  activity              Report typed activity, native lifetime, and the bounded durable approval summary
  wait                  Wait for one exact turn or native exit. Timeout and cancellation leave work running
  machines              
  find-session          Resolve an exact name within this gateway owner and machine. Reject duplicates
  name-session          
  label-session         
  remove-session-label  
  rename                
  label                 
  remove-label          
  revoke                Revoke remote access without terminating the device's native sessions
  history-streams       
  history               Read and verify one protected history page; data remains normalized/base64 in JSON
  sessions              
  roots                 
  browse                
  run                   Launch once using a persisted scoped key. Reconcile an uncertain result by that key
  reconcile             
  starts                List original journal identities for this gateway/owner/machine; never resends them
  send                  Send one explicitly controlled input; lost replies never cause a retry
  interrupt             
  terminate             
  replay                Read one bounded native output page/checkpoint. Does not acquire control
  help                  Print this message or the help of the given subcommand(s)

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote setup`

```text
Save connection settings, then use `remote` to sign in and open the TUI

Usage: qgenie-remote remote setup [OPTIONS]

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote profile`

```text
Manage local launch profiles without saving credentials

Usage: qgenie-remote remote profile [OPTIONS] <COMMAND>

Commands:
  save    Save a local profile. --gateway binds it to one service; no sign-in is needed
  list    
  show    
  remove  
  run     Launch once. Retain the printed key if the result is uncertain
  help    Print this message or the help of the given subcommand(s)

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote profile save`

```text
Save a local profile. --gateway binds it to one service; no sign-in is needed

Usage: qgenie-remote remote profile save [OPTIONS] --cwd <CWD> <NAME> <MACHINE> [-- <COMMAND>...]

Arguments:
  <NAME>
          

  <MACHINE>
          

  [COMMAND]...
          

Options:
      --cwd <CWD>
          

      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --agent <AGENT>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --task <TASK>
          

      --ca-bundle <CA_BUNDLE>
          

      --replace
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote profile list`

```text
Usage: qgenie-remote remote profile list [OPTIONS]

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote profile show`

```text
Usage: qgenie-remote remote profile show [OPTIONS] <NAME>

Arguments:
  <NAME>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote profile remove`

```text
Usage: qgenie-remote remote profile remove [OPTIONS] <NAME>

Arguments:
  <NAME>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote profile run`

```text
Launch once. Retain the printed key if the result is uncertain

Usage: qgenie-remote remote profile run [OPTIONS] <NAME>

Arguments:
  <NAME>
          

Options:
      --gateway <GATEWAY>
          

      --key <KEY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote wait-output`

```text
Wait for a literal in fresh PTY output after an explicit event cursor

Usage: qgenie-remote remote wait-output [OPTIONS] --after <AFTER> --literal <LITERAL> <MACHINE> <SESSION>

Arguments:
  <MACHINE>
          

  <SESSION>
          

Options:
      --after <AFTER>
          

      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --literal <LITERAL>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --timeout <TIMEOUT>
          [default: 300]

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote wait-many`

```text
Observe up to 16 exact targets. A resolved target may have failed or need approval

Usage: qgenie-remote remote wait-many [OPTIONS] --target <TARGET>

Options:
      --gateway <GATEWAY>
          

      --mode <MODE>
          [default: all]
          [possible values: any, all]

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --target <TARGET>
          Repeat MACHINE/SESSION/exit or MACHINE/SESSION/turn:TURN_ID

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --timeout <TIMEOUT>
          [default: 300]

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote read`

```text
Read a bounded plain-text checkpoint without controlling or scrolling the session

Usage: qgenie-remote remote read [OPTIONS] <MACHINE> <SESSION>

Arguments:
  <MACHINE>
          

  <SESSION>
          

Options:
      --gateway <GATEWAY>
          

      --lines <LINES>
          [default: 100]

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote events`

```text
Emit bounded JSON lines. Checkpoint records explicitly mark unavailable event history

Usage: qgenie-remote remote events [OPTIONS] <MACHINE> <SESSION>

Arguments:
  <MACHINE>
          

  <SESSION>
          

Options:
      --after <AFTER>
          [default: 0]

      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --limit <LIMIT>
          [default: 100]

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --timeout <TIMEOUT>
          [default: 10]

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote activity`

```text
Report typed activity, native lifetime, and the bounded durable approval summary

Usage: qgenie-remote remote activity [OPTIONS] <MACHINE> <SESSION>

Arguments:
  <MACHINE>
          

  <SESSION>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote wait`

```text
Wait for one exact turn or native exit. Timeout and cancellation leave work running

Usage: qgenie-remote remote wait [OPTIONS] <MACHINE> <SESSION>

Arguments:
  <MACHINE>
          

  <SESSION>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --turn <TURN>
          

      --exit
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --timeout <TIMEOUT>
          [default: 300]

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote machines`

```text
Usage: qgenie-remote remote machines [OPTIONS]

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote find-session`

```text
Resolve an exact name within this gateway owner and machine. Reject duplicates

Usage: qgenie-remote remote find-session [OPTIONS] <MACHINE> <NAME>

Arguments:
  <MACHINE>
          

  <NAME>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote name-session`

```text
Usage: qgenie-remote remote name-session [OPTIONS] <MACHINE> <SESSION> [NAME]

Arguments:
  <MACHINE>
          

  <SESSION>
          

  [NAME]
          

Options:
      --clear
          

      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote label-session`

```text
Usage: qgenie-remote remote label-session [OPTIONS] <MACHINE> <SESSION> <KEY> <VALUE>

Arguments:
  <MACHINE>
          

  <SESSION>
          

  <KEY>
          

  <VALUE>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote remove-session-label`

```text
Usage: qgenie-remote remote remove-session-label [OPTIONS] <MACHINE> <SESSION> <KEY>

Arguments:
  <MACHINE>
          

  <SESSION>
          

  <KEY>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote rename`

```text
Usage: qgenie-remote remote rename [OPTIONS] <MACHINE> <NAME>

Arguments:
  <MACHINE>
          

  <NAME>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote label`

```text
Usage: qgenie-remote remote label [OPTIONS] <MACHINE> <KEY> <VALUE>

Arguments:
  <MACHINE>
          

  <KEY>
          

  <VALUE>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote remove-label`

```text
Usage: qgenie-remote remote remove-label [OPTIONS] <MACHINE> <KEY>

Arguments:
  <MACHINE>
          

  <KEY>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote revoke`

```text
Revoke remote access without terminating the device's native sessions

Usage: qgenie-remote remote revoke [OPTIONS] <MACHINE>

Arguments:
  <MACHINE>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --yes
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote history-streams`

```text
Usage: qgenie-remote remote history-streams [OPTIONS] <SESSION>

Arguments:
  <SESSION>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote history`

```text
Read and verify one protected history page; data remains normalized/base64 in JSON

Usage: qgenie-remote remote history [OPTIONS] --stream-epoch <STREAM_EPOCH> <SESSION>

Arguments:
  <SESSION>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --stream-epoch <STREAM_EPOCH>
          

      --after <AFTER>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --limit <LIMIT>
          [default: 4]

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote sessions`

```text
Usage: qgenie-remote remote sessions [OPTIONS] <MACHINE>

Arguments:
  <MACHINE>
          

Options:
      --gateway <GATEWAY>
          

      --page
          Return one page with an explicit continuation cursor

      --state-dir <STATE_DIR>
          

      --after <AFTER>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --active
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --filter <FILTER>
          Match session ID, directory, name, or label text across the listed pages

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote roots`

```text
Usage: qgenie-remote remote roots [OPTIONS] <MACHINE>

Arguments:
  <MACHINE>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote browse`

```text
Usage: qgenie-remote remote browse [OPTIONS] <MACHINE> <PATH>

Arguments:
  <MACHINE>
          

  <PATH>
          

Options:
      --after <AFTER>
          

      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote run`

```text
Launch once using a persisted scoped key. Reconcile an uncertain result by that key

Usage: qgenie-remote remote run [OPTIONS] --cwd <CWD> <MACHINE> [-- <COMMAND>...]

Arguments:
  <MACHINE>
          

  [COMMAND]...
          

Options:
      --cwd <CWD>
          

      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --key <KEY>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --rows <ROWS>
          [default: 24]

      --ca-bundle <CA_BUNDLE>
          

      --cols <COLS>
          [default: 80]

      --agent <AGENT>
          

      --device
          Use a device code instead of opening a browser

      --resume-id <RESUME_ID>
          Resume a confirmed ended thread in a new owned session

      --task <TASK>
          Initial agent task. This text is not a permission boundary

  -h, --help
          Print help
```

## `qgenie-remote remote reconcile`

```text
Usage: qgenie-remote remote reconcile [OPTIONS] <MACHINE> <KEY>

Arguments:
  <MACHINE>
          

  <KEY>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote starts`

```text
List original journal identities for this gateway/owner/machine; never resends them

Usage: qgenie-remote remote starts [OPTIONS] <MACHINE>

Arguments:
  <MACHINE>
          

Options:
      --gateway <GATEWAY>
          

      --page
          

      --state-dir <STATE_DIR>
          

      --after <AFTER>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote send`

```text
Send one explicitly controlled input; lost replies never cause a retry

Usage: qgenie-remote remote send [OPTIONS] <MACHINE> <SESSION> [TEXT]

Arguments:
  <MACHINE>
          

  <SESSION>
          

  [TEXT]
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --stdin
          Read at most 32 KiB from stdin before acquiring control

      --enter
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --prompt
          

      --ca-bundle <CA_BUNDLE>
          

      --wait
          Observe this prompt's exact native turn after releasing control

      --device
          Use a device code instead of opening a browser

      --timeout <TIMEOUT>
          [default: 300]

      --steer
          

      --takeover
          

  -h, --help
          Print help
```

## `qgenie-remote remote interrupt`

```text
Usage: qgenie-remote remote interrupt [OPTIONS] <MACHINE> <SESSION>

Arguments:
  <MACHINE>
          

  <SESSION>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --takeover
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote terminate`

```text
Usage: qgenie-remote remote terminate [OPTIONS] <MACHINE> <SESSION>

Arguments:
  <MACHINE>
          

  <SESSION>
          

Options:
      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --yes
          

      --external-confirmed
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote remote replay`

```text
Read one bounded native output page/checkpoint. Does not acquire control

Usage: qgenie-remote remote replay [OPTIONS] <MACHINE> <SESSION>

Arguments:
  <MACHINE>
          

  <SESSION>
          

Options:
      --after <AFTER>
          [default: 0]

      --gateway <GATEWAY>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --token-file <TOKEN_FILE>
          

      --oauth-config <OAUTH_CONFIG>
          

      --ca-bundle <CA_BUNDLE>
          

      --device
          Use a device code instead of opening a browser

  -h, --help
          Print help
```

## `qgenie-remote tui`

```text
Start the interactive machine/session interface

Usage: qgenie-remote tui [OPTIONS]

Options:
      --session <SESSION>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote up`

```text
Start the independent local session daemon, if it is not running

Usage: qgenie-remote up [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote sessiond`

```text
Run the local session owner in the foreground

Usage: qgenie-remote sessiond [OPTIONS]

Options:
      --host-config <HOST_CONFIG>
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote status`

```text
Show local machine and daemon status

Usage: qgenie-remote status [OPTIONS]

Options:
      --json
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote machines`

```text
List available machines (the local machine in this preview)

Usage: qgenie-remote machines [OPTIONS]

Options:
      --json
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote list`

```text
List running and historical local sessions

Usage: qgenie-remote list [OPTIONS]

Options:
      --page
          

      --state-dir <STATE_DIR>
          

      --after <AFTER>
          

      --root <ROOT>
          

      --active
          

      --json
          

  -h, --help
          Print help
```

## `qgenie-remote run`

```text
Start a command in a PTY; keep its printed key to reconcile uncertainty

Usage: qgenie-remote run [OPTIONS] [-- <COMMAND>...]

Arguments:
  [COMMAND]...
          

Options:
      --cwd <CWD>
          

      --state-dir <STATE_DIR>
          

      --key <KEY>
          

      --root <ROOT>
          

      --rows <ROWS>
          [default: 24]

      --cols <COLS>
          [default: 80]

  -h, --help
          Print help
```

## `qgenie-remote attach`

```text
Open a session in observe mode; Ctrl-] c requests input control

Usage: qgenie-remote attach [OPTIONS] <SESSION>

Arguments:
  <SESSION>
          

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote reconcile`

```text
Look up the original start key without sending a new launch

Usage: qgenie-remote reconcile [OPTIONS] <KEY>

Arguments:
  <KEY>
          

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote send`

```text
Send one explicit input, release control, and never retry it automatically

Usage: qgenie-remote send [OPTIONS] <SESSION> <TEXT>

Arguments:
  <SESSION>
          

  <TEXT>
          

Options:
      --enter
          

      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

      --takeover
          

  -h, --help
          Print help
```

## `qgenie-remote interrupt`

```text
Interrupt the foreground turn without terminating the shell

Usage: qgenie-remote interrupt [OPTIONS] <SESSION>

Arguments:
  <SESSION>
          

Options:
      --state-dir <STATE_DIR>
          

      --takeover
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote terminate`

```text
Terminate a session explicitly

Usage: qgenie-remote terminate [OPTIONS] <SESSION>

Arguments:
  <SESSION>
          

Options:
      --state-dir <STATE_DIR>
          

      --yes
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote stop`

```text
Stop the daemon; refuse while sessions remain unless termination is explicit

Usage: qgenie-remote stop [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --terminate-sessions
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote doctor`

```text
Check local IPC, roots, capabilities, and preview limits

Usage: qgenie-remote doctor [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

## `qgenie-remote logs`

```text
Read daemon diagnostics. PTY content is not written to this log

Usage: qgenie-remote logs [OPTIONS]

Options:
      --state-dir <STATE_DIR>
          

      --root <ROOT>
          

  -h, --help
          Print help
```

