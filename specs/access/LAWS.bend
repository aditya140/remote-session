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

# ACCESS-003: No event can restore a revoked grant.
law revocation_step:
  for d: Nat
  for c: Bool
  for e: M.RenewalEvent
  {M.revoked(M.step(M.Grant{d, c, True{}}, e)) == True{} : Bool}

law revocation_trace:
  for es: List<&2, M.RenewalEvent>
  for +d: Nat
  for +c: Bool
  {M.revoked(M.replay(es, M.Grant{d, c, True{}})) == True{} : Bool}

# ACCESS clock: A failed clock requires a new grant, not reuse of the old grant.
law failed_clock_step:
  for d: Nat
  for r: Bool
  for e: M.RenewalEvent
  {M.clock_ok(M.step(M.Grant{d, False{}, r}, e)) == False{} : Bool}

law failed_clock_trace:
  for es: List<&2, M.RenewalEvent>
  for +d: Nat
  for +r: Bool
  {M.clock_ok(M.replay(es, M.Grant{d, False{}, r})) == False{} : Bool}
