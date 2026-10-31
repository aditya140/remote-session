import Base

# The two hold bits denote coverage by distinct committed holds for one item.
type Item is Data:
  Item{exists: Bool, hold_a: Bool, hold_b: Bool, expired: Bool}

type HoldEvent is Data:
  PlaceA{}
  PlaceB{}
  ReleaseA{}
  ReleaseB{}
  Purge{}
  UploadRetry{}

def purge(exists: Bool, a: Bool, b: Bool, expired: Bool) -> Bool:
  match a b expired:
    case False{} False{} True{}: False{}
    case _ _ _: exists

# Each transition represents one serialized lifecycle transaction.
def step(s: Item, e: HoldEvent) -> Item:
  match s e:
    case Item{x, a, b, age} PlaceA{}: Item{x, True{}, b, age}
    case Item{x, a, b, age} PlaceB{}: Item{x, a, True{}, age}
    case Item{x, a, b, age} ReleaseA{}: Item{x, False{}, b, age}
    case Item{x, a, b, age} ReleaseB{}: Item{x, a, False{}, age}
    case Item{+x, +a, +b, +age} Purge{}: Item{purge(x, a, b, age), a, b, age}
    case s0 UploadRetry{}: s0
