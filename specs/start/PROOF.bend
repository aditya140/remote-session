import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def identity_rebuild(+f: Nat, +s: Nat, pair: M.Phase & M.Effect) -> {M.identity(Pair.fst(M.Record, M.Effect, M.rebuild(f, s, pair))) == (f, s) : Nat & Nat}:
  (phase, effect) = pair
  {==}

def Laws.identity_step(r, e):
  match r:
    case M.Record{f, s, p}: identity_rebuild(f, s, M.phase_step(p, e))

def Laws.identity_trace(es, r):
  match es:
    case Nil{}: {==}
    case Con{+e, tail}:
      Equal.trans(Nat & Nat, M.identity(M.replay(tail, M.next(r, e))), M.identity(M.next(r, e)), M.identity(r), Laws.identity_trace(tail, M.next(r, e)), Laws.identity_step(r, e))

def Laws.unknown_no_relaunch(): {==}

def Laws.accepted_launch(): {==}

def Laws.ambiguous_crash(): {==}

def Laws.lookup_read_only(p):
  match p:
    case M.Accepted{}: {==}
    case M.Launching{}: {==}
    case M.Running{}: {==}
    case M.Failed{}: {==}
    case M.Unknown{}: {==}
    case M.SessionLost{}: {==}

def Laws.launch_budget(es, p):
  match es:
    case Nil{}: {==}
    case Con{+e, tail}:
      match e p:
        case M.Dispatch{} M.Accepted{}: Laws.launch_budget(tail, M.Launching{})
        case M.NativeRunning{} M.Accepted{}: Laws.launch_budget(tail, M.Accepted{})
        case M.NativeFailed{} M.Accepted{}: Laws.launch_budget(tail, M.Accepted{})
        case M.NativeLost{} M.Accepted{}: Laws.launch_budget(tail, M.Accepted{})
        case M.ReplyLost{} M.Accepted{}: Laws.launch_budget(tail, M.Accepted{})
        case M.Crash{} M.Accepted{}: Laws.launch_budget(tail, M.Accepted{})
        case M.Lookup{} M.Accepted{}: Laws.launch_budget(tail, M.Accepted{})
        case M.Dispatch{} M.Launching{}: Laws.launch_budget(tail, M.Launching{})
        case M.NativeRunning{} M.Launching{}: Laws.launch_budget(tail, M.Running{})
        case M.NativeFailed{} M.Launching{}: Laws.launch_budget(tail, M.Failed{})
        case M.NativeLost{} M.Launching{}: Laws.launch_budget(tail, M.Launching{})
        case M.ReplyLost{} M.Launching{}: Laws.launch_budget(tail, M.Launching{})
        case M.Crash{} M.Launching{}: Laws.launch_budget(tail, M.Unknown{})
        case M.Lookup{} M.Launching{}: Laws.launch_budget(tail, M.Launching{})
        case M.Dispatch{} M.Running{}: Laws.launch_budget(tail, M.Running{})
        case M.NativeRunning{} M.Running{}: Laws.launch_budget(tail, M.Running{})
        case M.NativeFailed{} M.Running{}: Laws.launch_budget(tail, M.Running{})
        case M.NativeLost{} M.Running{}: Laws.launch_budget(tail, M.SessionLost{})
        case M.ReplyLost{} M.Running{}: Laws.launch_budget(tail, M.Running{})
        case M.Crash{} M.Running{}: Laws.launch_budget(tail, M.Unknown{})
        case M.Lookup{} M.Running{}: Laws.launch_budget(tail, M.Running{})
        case M.Dispatch{} M.Failed{}: Laws.launch_budget(tail, M.Failed{})
        case M.NativeRunning{} M.Failed{}: Laws.launch_budget(tail, M.Failed{})
        case M.NativeFailed{} M.Failed{}: Laws.launch_budget(tail, M.Failed{})
        case M.NativeLost{} M.Failed{}: Laws.launch_budget(tail, M.Failed{})
        case M.ReplyLost{} M.Failed{}: Laws.launch_budget(tail, M.Failed{})
        case M.Crash{} M.Failed{}: Laws.launch_budget(tail, M.Failed{})
        case M.Lookup{} M.Failed{}: Laws.launch_budget(tail, M.Failed{})
        case M.Dispatch{} M.Unknown{}: Laws.launch_budget(tail, M.Unknown{})
        case M.NativeRunning{} M.Unknown{}: Laws.launch_budget(tail, M.Running{})
        case M.NativeFailed{} M.Unknown{}: Laws.launch_budget(tail, M.Failed{})
        case M.NativeLost{} M.Unknown{}: Laws.launch_budget(tail, M.Unknown{})
        case M.ReplyLost{} M.Unknown{}: Laws.launch_budget(tail, M.Unknown{})
        case M.Crash{} M.Unknown{}: Laws.launch_budget(tail, M.Unknown{})
        case M.Lookup{} M.Unknown{}: Laws.launch_budget(tail, M.Unknown{})
        case M.Dispatch{} M.SessionLost{}: Laws.launch_budget(tail, M.SessionLost{})
        case M.NativeRunning{} M.SessionLost{}: Laws.launch_budget(tail, M.SessionLost{})
        case M.NativeFailed{} M.SessionLost{}: Laws.launch_budget(tail, M.SessionLost{})
        case M.NativeLost{} M.SessionLost{}: Laws.launch_budget(tail, M.SessionLost{})
        case M.ReplyLost{} M.SessionLost{}: Laws.launch_budget(tail, M.SessionLost{})
        case M.Crash{} M.SessionLost{}: Laws.launch_budget(tail, M.SessionLost{})
        case M.Lookup{} M.SessionLost{}: Laws.launch_budget(tail, M.SessionLost{})

def Laws.matching_fingerprint(f, s, p): C.eq_self(f)

def Laws.running_crash_requires_evidence(): {==}
