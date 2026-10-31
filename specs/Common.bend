import Base

def eq(a: Nat, b: Nat) -> Bool:
  match a b:
    case 0n 0n: True{}
    case 0n 1n+bp: False{}
    case 1n+ap 0n: False{}
    case 1n+ap 1n+bp: eq(ap, bp)

def le(a: Nat, b: Nat) -> Bool:
  match a b:
    case 0n b0: True{}
    case 1n+ap 0n: False{}
    case 1n+ap 1n+bp: le(ap, bp)

def lt(a: Nat, b: Nat) -> Bool:
  le(1n+a, b)

def eq_self(+n: Nat) -> {eq(n, n) == True{} : Bool}:
  match n:
    case 0n: {==}
    case 1n+p: eq_self(p)

def le_self(+n: Nat) -> {le(n, n) == True{} : Bool}:
  match n:
    case 0n: {==}
    case 1n+p: le_self(p)

def lt_self(+n: Nat) -> {lt(n, n) == False{} : Bool}:
  match n:
    case 0n: {==}
    case 1n+p: lt_self(p)
