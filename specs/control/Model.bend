import Base
import ../Common.bend as C

type Lease is Data:
  Lease{holder: Nat, host: Nat, generation: Nat, activated: Bool, lease_live: Bool, access_live: Bool}

def admitted(s: Lease, who: Nat, host: Nat, generation: Nat) -> Bool:
  match s:
    case Lease{holder, current_host, current_generation, active, lease_live, access_live}:
      Bool.and(active, Bool.and(lease_live, Bool.and(access_live,
        Bool.and(C.eq(holder, who), Bool.and(C.eq(current_host, host), C.eq(current_generation, generation))))))

def reconnect(s: Lease) -> Lease:
  match s:
    case Lease{h, host, gen, active, lease_live, access_live}:
      Lease{h, host, gen, False{}, lease_live, access_live}

# Compare the incoming activation with the durable floor before this step.
def activation(newer: Bool, confirmed: Bool, current: Lease, proposed: Lease) -> Lease:
  match newer confirmed:
    case True{} True{}: proposed
    case _ _: current

def retry_input(uncertain: Bool) -> Bool:
  match uncertain:
    case True{}: False{}
    case False{}: False{}

def activate(current: Lease, proposed: Lease, confirmed: Bool) -> Lease:
  match current proposed:
    case Lease{+holder, +host, +gen, +active, +live, +access} Lease{+nh, +nHost, +ng, +na, +nl, +nx}:
      activation(C.lt(gen, ng), confirmed, Lease{holder, host, gen, active, live, access}, Lease{nh, nHost, ng, na, nl, nx})
