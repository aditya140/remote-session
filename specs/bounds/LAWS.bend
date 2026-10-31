import Base
import ./Model.bend as M
import ../Common.bend as C

# BOUNDS-001
law full_rejects_without_wrap:
  for +used: Nat
  {M.take(M.Budget{used, 0n}) == (M.Budget{used, 0n}, M.Full{}) : M.Budget & M.Admission}

# BOUNDS-002
law available_slot_admits:
  for +used: Nat
  for +left: Nat
  {M.take(M.Budget{used, 1n+left}) == (M.Budget{1n+used, left}, M.Admitted{}) : M.Budget & M.Admission}

# BOUNDS-003
law total_preserved:
  for +b: M.Budget
  {M.total(M.after(b)) == M.total(b) : Nat}

# BOUNDS-003 trace
law requests_preserve_bound:
  for n: Nat
  for +b: M.Budget
  {M.total(M.requests(n, b)) == M.total(b) : Nat}
