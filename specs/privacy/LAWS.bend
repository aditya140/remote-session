import Base
import ./Model.bend as M
import ../Common.bend as C

# PRIVACY-001 two observations
law hold_and_audit_hidden:
  for +content: Nat
  for held_a: Bool
  for reads_a: Nat
  for held_b: Bool
  for reads_b: Nat
  {M.owner_view(M.History{content, held_a, reads_a}) == M.owner_view(M.History{content, held_b, reads_b}) : Nat}

# PRIVACY-002
law other_owner_cannot_read:
  for s: M.History
  {M.owner_read(False{}, s) == None{} : Maybe<Nat>}

# PRIVACY-003
law no_audit_no_admin_release:
  for held: Bool
  for case_matches: Bool
  {M.admin_read(True{}, True{}, False{}, held, case_matches) == False{} : Bool}

# PRIVACY-004
law held_read_needs_case:
  {M.admin_read(True{}, True{}, True{}, True{}, False{}) == False{} : Bool}

# PRIVACY-005
law audited_eligible_admin_reads:
  {M.admin_read(True{}, True{}, True{}, True{}, True{}) == True{} : Bool}
