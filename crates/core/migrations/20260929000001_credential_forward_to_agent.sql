-- Adds a non-secret metadata flag marking that a credential's private key
-- should be loaded into the local ssh-agent and forwarded to the remote
-- server via ForwardAgent (auth-agent@openssh.com).
ALTER TABLE credential_entries
    ADD COLUMN forward_to_agent INTEGER NOT NULL DEFAULT 0;