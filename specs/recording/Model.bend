import Base

# Digest stands for immutable normalized content identity, not a hash proof.
type Chunk is Data:
  Chunk{identity: Nat, digest: Nat, capture_time: Nat}

type Storage is Data:
  Retained{}
  GapCommitted{}
  UncertainLoss{}
  Reclaimed{}

type StorageEvent is Data:
  Receipt{identity_matches: Bool, epoch_matches: Bool, durable: Bool}
  Capacity{gap_durable: Bool}
  MetadataFailure{}
  RetryUpload{}

def receipt(i: Bool, e: Bool, d: Bool) -> Storage:
  match i e d:
    case True{} True{} True{}: Reclaimed{}
    case _ _ _: Retained{}

def capacity(gap: Bool) -> Storage:
  match gap:
    case True{}: GapCommitted{}
    case False{}: UncertainLoss{}

def step(s: Storage, e: StorageEvent) -> Storage:
  match s e:
    case Retained{} Receipt{i, epoch, durable}: receipt(i, epoch, durable)
    case Retained{} Capacity{gap}: capacity(gap)
    case Retained{} MetadataFailure{}: UncertainLoss{}
    case s0 _: s0

def upload(c: Chunk) -> Chunk:
  c

def stops_execution(e: StorageEvent) -> Bool:
  False{}
