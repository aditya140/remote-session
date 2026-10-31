import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.missing_client_denies(g, h, a): {==}

def Laws.missing_host_denies(a): {==}

def Laws.unverified_adapter_denies(): {==}

def Laws.all_participants_enable(): {==}

def Laws.missing_required_rejects_handshake(): {==}
