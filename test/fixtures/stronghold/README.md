# Pre-update Stronghold snapshot

`upstream-2.1.0.snapshot` was generated with unmodified registry
`iota_stronghold 2.1.0`, `iota-crypto 0.23.2`, and `k256 0.13.4`, outside the
PortMate workspace patch. It is a synthetic fixture, not user data.

- SHA-256: `114586ada3dd289efa16d21f9e7a847bc882aedce98f5196659d77bace34163a`
- Raw snapshot key: 32 bytes, each `0x07` (not a real password).
- Client: `portmate-portable-vault`.
- Account/value: `fixture-account` / `synthetic-vault-value-not-a-real-secret`.

Generation uses the normal `Stronghold::create_client`, `client.store().insert`
and `commit_with_keyprovider` APIs; no custom encryption or snapshot encoder.
The regression copies the fixture into a disposable directory, rejects a wrong
key, reads the old value, saves a new record and reopens both records using the
updated dependency graph. The checked-in fixture is never rewritten by tests.
