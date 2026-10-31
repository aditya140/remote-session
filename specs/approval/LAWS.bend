import Base
import ./Model.bend as M
import ../Common.bend as C

# APPROVAL-001
law winner_not_overwritten:
  for +first: Nat
  for second: Nat
  for +a: M.Application
  {M.step(M.Won{first, a}, M.Commit{second}) == (M.Won{first, a}, M.NoEffect{}) : M.State & M.Effect}

# APPROVAL-002
law atomic_commit:
  for +d: Nat
  {M.step(M.Pending{}, M.Commit{d}) == (M.Won{d, M.Queued{}}, M.CommitWinnerAuditOutbox{}) : M.State & M.Effect}

# APPROVAL-003
law lost_reply_unknown:
  {M.application_step(M.Submitted{}, M.ReplyLost{}) == (M.Unknown{}, M.NoEffect{}) : M.Application & M.Effect}

# APPROVAL-004
law unknown_no_resubmit:
  for allowed: Bool
  {M.application_step(M.Unknown{}, M.Deliver{allowed}) == (M.Unknown{}, M.NoEffect{}) : M.Application & M.Effect}

# APPROVAL-004 policy
law policy_cancel:
  {M.application_step(M.Queued{}, M.Deliver{False{}}) == (M.Cancelled{}, M.NoEffect{}) : M.Application & M.Effect}

# APPROVAL positive
law eligible_delivery:
  {M.application_step(M.Queued{}, M.Deliver{True{}}) == (M.Submitted{}, M.SubmitNative{}) : M.Application & M.Effect}

# APPROVAL-004: No unsupported repeat submission over any finite trace.
law submission_budget:
  for es: List<&2, M.StepEvent>
  for +a: M.Application
  {C.le(M.count(es, a), M.remaining(a)) == True{} : Bool}

# APPROVAL-001: An established winner is stable under every subsequent event.
law winner_trace:
  for es: List<&2, M.StepEvent>
  for +d: Nat
  for a: M.Application
  {M.winner(M.state_trace(es, M.Won{d, a})) == Some{d} : Maybe<Nat>}
