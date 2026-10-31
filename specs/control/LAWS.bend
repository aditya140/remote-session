import Base
import ./Model.bend as M
import ../Common.bend as C

# CONTROL-001 activation
law unactivated_denied:
  for h: Nat
  for host: Nat
  for gen: Nat
  for live: Bool
  for access: Bool
  for who: Nat
  for rh: Nat
  for rg: Nat
  {M.admitted(M.Lease{h, host, gen, False{}, live, access}, who, rh, rg) == False{} : Bool}

# CONTROL-001 expiry
law expired_access_denied:
  for h: Nat
  for host: Nat
  for gen: Nat
  for who: Nat
  for rh: Nat
  for rg: Nat
  {M.admitted(M.Lease{h, host, gen, True{}, True{}, False{}}, who, rh, rg) == False{} : Bool}

# CONTROL-001 positive
law active_holder_allowed:
  for +h: Nat
  for +host: Nat
  for +gen: Nat
  {M.admitted(M.Lease{h, host, gen, True{}, True{}, True{}}, h, host, gen) == True{} : Bool}

# CONTROL-002
law stale_activation_ignored:
  for confirmed: Bool
  for +current: M.Lease
  for proposed: M.Lease
  {M.activation(False{}, confirmed, current, proposed) == current : M.Lease}

# CONTROL-003
law no_unconfirmed_takeover:
  for +current: M.Lease
  for proposed: M.Lease
  {M.activation(True{}, False{}, current, proposed) == current : M.Lease}

# CONTROL-003 reconnect
law reconnect_observes:
  for s: M.Lease
  for who: Nat
  for rh: Nat
  for rg: Nat
  {M.admitted(M.reconnect(s), who, rh, rg) == False{} : Bool}

# CONTROL-004
law uncertain_input_not_resent:
  {M.retry_input(True{}) == False{} : Bool}

# CONTROL-002: Equal generations cannot replace the current holder.
law same_generation_cannot_takeover:
  for +g: Nat
  for +current: Nat
  for proposed: Nat
  {M.activate(M.Lease{current, 1n, g, True{}, True{}, True{}}, M.Lease{proposed, 1n, g, True{}, True{}, True{}}, True{}) == M.Lease{current, 1n, g, True{}, True{}, True{}} : M.Lease}

# CONTROL-001: Each admission guard must independently reject invalid authority.
law expired_lease_denied:
  for h: Nat
  for host: Nat
  for gen: Nat
  for access: Bool
  for who: Nat
  for rh: Nat
  for rg: Nat
  {M.admitted(M.Lease{h, host, gen, True{}, False{}, access}, who, rh, rg) == False{} : Bool}

law wrong_holder_denied:
  for +holder: Nat
  for +who: Nat
  for +host: Nat
  for +gen: Nat
  for different: {C.eq(holder, who) == False{} : Bool}
  {M.admitted(M.Lease{holder, host, gen, True{}, True{}, True{}}, who, host, gen) == False{} : Bool}

law wrong_host_denied:
  for +holder: Nat
  for +host: Nat
  for +requested: Nat
  for +gen: Nat
  for different: {C.eq(host, requested) == False{} : Bool}
  {M.admitted(M.Lease{holder, host, gen, True{}, True{}, True{}}, holder, requested, gen) == False{} : Bool}

law wrong_controller_generation_denied:
  for +holder: Nat
  for +host: Nat
  for +gen: Nat
  for +requested: Nat
  for different: {C.eq(gen, requested) == False{} : Bool}
  {M.admitted(M.Lease{holder, host, gen, True{}, True{}, True{}}, holder, host, requested) == False{} : Bool}

# CONTROL-003: A valid confirmed proposal must be able to advance.
law eligible_activation_applied:
  for current: M.Lease
  for +proposed: M.Lease
  {M.activation(True{}, True{}, current, proposed) == proposed : M.Lease}
