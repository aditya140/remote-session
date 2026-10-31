import Base
import ./Model.bend as M
import ../Common.bend as C

# PROTO-001
law round_trip:
  for +m: M.Message
  {M.decode(M.encode(m)) == Some{m} : Maybe<M.Message>}

# PROTO-002
law invalid_frame_no_dispatch:
  for ts: List<M.Token>
  {M.dispatch(False{}, ts) == None{} : Maybe<M.Message>}

# PROTO-003
law truncated_input_denied:
  for s: Nat
  for n: Nat
  for h: Nat
  {M.decode([M.InputTag{}, M.Number{s}, M.Number{n}, M.Number{h}]) == None{} : Maybe<M.Message>}

# PROTO-004
law no_tag_no_dispatch:
  for n: Nat
  {M.decode([M.Number{n}]) == None{} : Maybe<M.Message>}
