import Base
import ../Common.bend as C

type Disposition is Data:
  Duplicate{}
  Apply{}
  Resync{}

def classify(c: Cmp) -> Disposition:
  match c:
    case LT{}: Resync{}
    case EQ{}: Apply{}
    case GT{}: Duplicate{}

# A contiguous cursor includes a snapshot's state through that cursor.
def receive(cursor: Nat, sequence: Nat) -> Disposition:
  classify(Nat.cmp(1n+cursor, sequence))

def checkpoint(snapshot: Nat) -> Nat:
  snapshot

def overflow() -> Disposition:
  Resync{}

def missing_tail(head: Nat, rendered: Nat) -> Bool:
  C.lt(rendered, head)
