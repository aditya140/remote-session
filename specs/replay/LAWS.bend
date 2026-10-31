import Base
import ./Model.bend as M
import ../Common.bend as C

# REPLAY-001
law snapshot_boundary:
  for +s: Nat
  {M.checkpoint(s) == s : Nat}

# REPLAY-002
law next_event_applies:
  for +s: Nat
  {M.receive(s, 1n+s) == M.Apply{} : M.Disposition}

# REPLAY-003
law snapshot_not_reapplied:
  for +s: Nat
  {M.receive(s, s) == M.Duplicate{} : M.Disposition}

# REPLAY-004
law gap_resync:
  for +s: Nat
  {M.receive(s, 1n+1n+s) == M.Resync{} : M.Disposition}

# REPLAY-005
law overflow_resync:
  {M.overflow() == M.Resync{} : M.Disposition}

# REPLAY-006
law lost_final_event_detected:
  for +s: Nat
  {M.missing_tail(1n+s, s) == True{} : Bool}
