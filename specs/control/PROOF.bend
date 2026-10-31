import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.unactivated_denied(h, host, gen, live, access, who, rh, rg): {==}

def Laws.expired_access_denied(h, host, gen, who, rh, rg): {==}

def and_true(a: Bool, b: Bool, pa: {a == True{} : Bool}, pb: {b == True{} : Bool}) -> {Bool.and(a, b) == True{} : Bool}:
  %pa : {Bool.and(a, b) == Bool.and(_, True{}) : Bool}
  %pb : {Bool.and(a, b) == Bool.and(a, _) : Bool}
  {==}

def Laws.active_holder_allowed(h, host, gen):
  and_true(C.eq(h, h), Bool.and(C.eq(host, host), C.eq(gen, gen)), C.eq_self(h), and_true(C.eq(host, host), C.eq(gen, gen), C.eq_self(host), C.eq_self(gen)))

def Laws.stale_activation_ignored(confirmed, current, proposed): {==}

def Laws.no_unconfirmed_takeover(current, proposed): {==}

def Laws.reconnect_observes(s, who, rh, rg):
  match s:
    case M.Lease{h, host, gen, a, l, x}: {==}

def Laws.uncertain_input_not_resent(): {==}

def activation_equal(+g: Nat, +current: M.Lease, proposed: M.Lease, e: {C.lt(g, g) == False{} : Bool}) -> {M.activation(C.lt(g, g), True{}, current, proposed) == current : M.Lease}:
  %Equal.sym(Bool, C.lt(g, g), False{}, e) : {M.activation(_, True{}, current, proposed) == current : M.Lease}
  {==}

def Laws.same_generation_cannot_takeover(g, current, proposed):
  activation_equal(g, M.Lease{current, 1n, g, True{}, True{}, True{}}, M.Lease{proposed, 1n, g, True{}, True{}, True{}}, C.lt_self(g))
