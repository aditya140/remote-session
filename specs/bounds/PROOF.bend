import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def add_succ(+a: Nat, +b: Nat) -> {1n+Nat.add(a, b) == Nat.add(a, 1n+b) : Nat}:
  match a:
    case 0n: {==}
    case 1n+p:
      %add_succ(p, b) : {1n+1n+Nat.add(p, b) == 1n+_ : Nat}
      {==}

def Laws.full_rejects_without_wrap(used): {==}

def Laws.available_slot_admits(used, left): {==}

def Laws.total_preserved(b):
  match b:
    case M.Budget{used, 0n}: {==}
    case M.Budget{used, 1n+left}: add_succ(used, left)

def Laws.requests_preserve_bound(n, b):
  match n:
    case 0n: {==}
    case 1n+p:
      Equal.trans(Nat, M.total(M.requests(p, M.after(b))), M.total(M.after(b)), M.total(b), Laws.requests_preserve_bound(p, M.after(b)), Laws.total_preserved(b))
