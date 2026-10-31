import Base

# Used and remaining form a bounded counter. No modular arithmetic is used.
type Budget is Data:
  Budget{used: Nat, remaining: Nat}

type Admission is Data:
  Full{}
  Admitted{}

def take(b: Budget) -> Budget & Admission:
  match b:
    case Budget{used, 0n}: (Budget{used, 0n}, Full{})
    case Budget{used, 1n+left}: (Budget{1n+used, left}, Admitted{})

def total(b: Budget) -> Nat:
  match b:
    case Budget{used, remaining}: Nat.add(used, remaining)

def after(b: Budget) -> Budget:
  Pair.fst(Budget, Admission, take(b))

def requests(n: Nat, b: Budget) -> Budget:
  match n:
    case 0n: b
    case 1n+p: requests(p, after(b))
