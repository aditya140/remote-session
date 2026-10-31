import Base
import ../Common.bend as C

# The deadline belongs to the original grant, in one local monotonic clock.
type Grant is Data:
  Grant{deadline: Nat, clock_ok: Bool, revoked: Bool}

def allowed(g: Grant, now: Nat) -> Bool:
  match g:
    case Grant{deadline, clock_ok, revoked}:
      Bool.and(clock_ok, Bool.and(Bool.not(revoked), C.lt(now, deadline)))

type RenewalEvent is Data:
  DelayedReply{}
  Retry{}
  TokenRefresh{}
  RedisHint{}
  DirectorySuccess{}
  Revoke{}
  ClockFailure{}

def step(g: Grant, e: RenewalEvent) -> Grant:
  match g e:
    case Grant{d, c, r} Revoke{}: Grant{d, c, True{}}
    case Grant{d, c, r} ClockFailure{}: Grant{d, False{}, r}
    case g0 _: g0

def deadline(g: Grant) -> Nat:
  match g:
    case Grant{d, c, r}: d

def replay(es: List<&2, RenewalEvent>, g: Grant) -> Grant:
  match es:
    case Nil{}: g
    case Con{+e, tail}: replay(tail, step(g, e))
