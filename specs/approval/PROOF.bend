import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.winner_not_overwritten(first, second, a):
  match a:
    case M.Queued{}: {==}
    case M.Submitted{}: {==}
    case M.Applied{}: {==}
    case M.Cancelled{}: {==}
    case M.Unknown{}: {==}

def Laws.atomic_commit(d): {==}

def Laws.lost_reply_unknown(): {==}

def Laws.unknown_no_resubmit(allowed): {==}

def Laws.policy_cancel(): {==}

def Laws.eligible_delivery(): {==}

def cancelled_zero(es: List<&2, M.StepEvent>) -> {M.count(es, M.Cancelled{}) == 0n : Nat}:
  match es:
    case Nil{}: {==}
    case Con{e, tail}: cancelled_zero(tail)

def cancelled_within_one(es: List<&2, M.StepEvent>) -> {C.le(M.count(es, M.Cancelled{}), 1n) == True{} : Bool}:
  Equal.cong(Nat, Bool, n => C.le(n, 1n), M.count(es, M.Cancelled{}), 0n, cancelled_zero(es))

def Laws.submission_budget(es, a):
  match es:
    case Nil{}: {==}
    case Con{e, tail}:
      match e a:
        case M.Commit{other} M.Queued{}: Laws.submission_budget(tail, M.Queued{})
        case M.Deliver{True{}} M.Queued{}: Laws.submission_budget(tail, M.Submitted{})
        case M.Deliver{False{}} M.Queued{}: cancelled_within_one(tail)
        case M.NativeApplied{} M.Queued{}: Laws.submission_budget(tail, M.Queued{})
        case M.ReplyLost{} M.Queued{}: Laws.submission_budget(tail, M.Queued{})
        case M.Cancel{} M.Queued{}: cancelled_within_one(tail)
        case M.Commit{other} M.Submitted{}: Laws.submission_budget(tail, M.Submitted{})
        case M.Deliver{True{}} M.Submitted{}: Laws.submission_budget(tail, M.Submitted{})
        case M.Deliver{False{}} M.Submitted{}: Laws.submission_budget(tail, M.Submitted{})
        case M.NativeApplied{} M.Submitted{}: Laws.submission_budget(tail, M.Applied{})
        case M.ReplyLost{} M.Submitted{}: Laws.submission_budget(tail, M.Unknown{})
        case M.Cancel{} M.Submitted{}: Laws.submission_budget(tail, M.Submitted{})
        case M.Commit{other} M.Applied{}: Laws.submission_budget(tail, M.Applied{})
        case M.Deliver{True{}} M.Applied{}: Laws.submission_budget(tail, M.Applied{})
        case M.Deliver{False{}} M.Applied{}: Laws.submission_budget(tail, M.Applied{})
        case M.NativeApplied{} M.Applied{}: Laws.submission_budget(tail, M.Applied{})
        case M.ReplyLost{} M.Applied{}: Laws.submission_budget(tail, M.Applied{})
        case M.Cancel{} M.Applied{}: Laws.submission_budget(tail, M.Applied{})
        case M.Commit{other} M.Cancelled{}: Laws.submission_budget(tail, M.Cancelled{})
        case M.Deliver{True{}} M.Cancelled{}: Laws.submission_budget(tail, M.Cancelled{})
        case M.Deliver{False{}} M.Cancelled{}: Laws.submission_budget(tail, M.Cancelled{})
        case M.NativeApplied{} M.Cancelled{}: Laws.submission_budget(tail, M.Cancelled{})
        case M.ReplyLost{} M.Cancelled{}: Laws.submission_budget(tail, M.Cancelled{})
        case M.Cancel{} M.Cancelled{}: Laws.submission_budget(tail, M.Cancelled{})
        case M.Commit{other} M.Unknown{}: Laws.submission_budget(tail, M.Unknown{})
        case M.Deliver{True{}} M.Unknown{}: Laws.submission_budget(tail, M.Unknown{})
        case M.Deliver{False{}} M.Unknown{}: Laws.submission_budget(tail, M.Unknown{})
        case M.NativeApplied{} M.Unknown{}: Laws.submission_budget(tail, M.Applied{})
        case M.ReplyLost{} M.Unknown{}: Laws.submission_budget(tail, M.Unknown{})
        case M.Cancel{} M.Unknown{}: Laws.submission_budget(tail, M.Unknown{})

def Laws.winner_trace(es, d, a):
  match es:
    case Nil{}: {==}
    case Con{e, tail}:
      match e a:
        case M.Commit{other} M.Queued{}: Laws.winner_trace(tail, d, M.Queued{})
        case M.Deliver{True{}} M.Queued{}: Laws.winner_trace(tail, d, M.Submitted{})
        case M.Deliver{False{}} M.Queued{}: Laws.winner_trace(tail, d, M.Cancelled{})
        case M.NativeApplied{} M.Queued{}: Laws.winner_trace(tail, d, M.Queued{})
        case M.ReplyLost{} M.Queued{}: Laws.winner_trace(tail, d, M.Queued{})
        case M.Cancel{} M.Queued{}: Laws.winner_trace(tail, d, M.Cancelled{})
        case M.Commit{other} M.Submitted{}: Laws.winner_trace(tail, d, M.Submitted{})
        case M.Deliver{True{}} M.Submitted{}: Laws.winner_trace(tail, d, M.Submitted{})
        case M.Deliver{False{}} M.Submitted{}: Laws.winner_trace(tail, d, M.Submitted{})
        case M.NativeApplied{} M.Submitted{}: Laws.winner_trace(tail, d, M.Applied{})
        case M.ReplyLost{} M.Submitted{}: Laws.winner_trace(tail, d, M.Unknown{})
        case M.Cancel{} M.Submitted{}: Laws.winner_trace(tail, d, M.Submitted{})
        case M.Commit{other} M.Applied{}: Laws.winner_trace(tail, d, M.Applied{})
        case M.Deliver{True{}} M.Applied{}: Laws.winner_trace(tail, d, M.Applied{})
        case M.Deliver{False{}} M.Applied{}: Laws.winner_trace(tail, d, M.Applied{})
        case M.NativeApplied{} M.Applied{}: Laws.winner_trace(tail, d, M.Applied{})
        case M.ReplyLost{} M.Applied{}: Laws.winner_trace(tail, d, M.Applied{})
        case M.Cancel{} M.Applied{}: Laws.winner_trace(tail, d, M.Applied{})
        case M.Commit{other} M.Cancelled{}: Laws.winner_trace(tail, d, M.Cancelled{})
        case M.Deliver{True{}} M.Cancelled{}: Laws.winner_trace(tail, d, M.Cancelled{})
        case M.Deliver{False{}} M.Cancelled{}: Laws.winner_trace(tail, d, M.Cancelled{})
        case M.NativeApplied{} M.Cancelled{}: Laws.winner_trace(tail, d, M.Cancelled{})
        case M.ReplyLost{} M.Cancelled{}: Laws.winner_trace(tail, d, M.Cancelled{})
        case M.Cancel{} M.Cancelled{}: Laws.winner_trace(tail, d, M.Cancelled{})
        case M.Commit{other} M.Unknown{}: Laws.winner_trace(tail, d, M.Unknown{})
        case M.Deliver{True{}} M.Unknown{}: Laws.winner_trace(tail, d, M.Unknown{})
        case M.Deliver{False{}} M.Unknown{}: Laws.winner_trace(tail, d, M.Unknown{})
        case M.NativeApplied{} M.Unknown{}: Laws.winner_trace(tail, d, M.Applied{})
        case M.ReplyLost{} M.Unknown{}: Laws.winner_trace(tail, d, M.Unknown{})
        case M.Cancel{} M.Unknown{}: Laws.winner_trace(tail, d, M.Unknown{})
