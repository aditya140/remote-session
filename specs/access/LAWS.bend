import Base
import ./Model.bend as M
import ../Common.bend as C

# ACCESS-001
law revoked_denied:
  for d: Nat
  for now: Nat
  {M.allowed(M.Grant{d, True{}, True{}}, now) == False{} : Bool}

# ACCESS-002
law deadline_step:
  for +g: M.Grant
  for e: M.RenewalEvent
  {M.deadline(M.step(g, e)) == M.deadline(g) : Nat}

# ACCESS-002 trace
law deadline_trace:
  for es: List<&2, M.RenewalEvent>
  for +g: M.Grant
  {M.deadline(M.replay(es, g)) == M.deadline(g) : Nat}

# ACCESS-003
law directory_cannot_unrevoke:
  for +d: Nat
  {M.step(M.Grant{d, True{}, True{}}, M.DirectorySuccess{}) == M.Grant{d, True{}, True{}} : M.Grant}

# ACCESS clock
law clock_failure_denies:
  for d: Nat
  for r: Bool
  for now: Nat
  {M.allowed(M.Grant{d, False{}, r}, now) == False{} : Bool}

# ACCESS deadline
law deadline_exclusive:
  for +d: Nat
  {M.allowed(M.Grant{d, True{}, False{}}, d) == False{} : Bool}

# ACCESS positive
law before_deadline:
  {M.allowed(M.Grant{1n, True{}, False{}}, 0n) == True{} : Bool}
