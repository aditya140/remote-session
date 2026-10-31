import Base
import ./Model.bend as M
import ../Common.bend as C

# CAP-001: The gateway must also support the capability.
law missing_gateway_denies:
  for h: Bool
  for a: Bool
  {M.enabled(True{}, False{}, h, a) == False{} : Bool}

# CAP-001
law missing_client_denies:
  for g: Bool
  for h: Bool
  for a: Bool
  {M.enabled(False{}, g, h, a) == False{} : Bool}

# CAP-002
law missing_host_denies:
  for a: Bool
  {M.enabled(True{}, True{}, False{}, a) == False{} : Bool}

# CAP-003
law unverified_adapter_denies:
  {M.enabled(True{}, True{}, True{}, False{}) == False{} : Bool}

# CAP-004
law all_participants_enable:
  {M.enabled(True{}, True{}, True{}, True{}) == True{} : Bool}

# CAP-005
law missing_required_rejects_handshake:
  {M.negotiate(True{}, False{}) == False{} : Bool}
