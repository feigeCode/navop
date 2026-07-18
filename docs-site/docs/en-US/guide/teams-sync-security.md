# Teams, sync, and encryption security

Navop supports personal sync and team-shared connection configuration. The website handles OTP sign-in, teams, and membership; the desktop application performs local encryption, decryption, and actual connection use. Cloud storage contains locally produced ciphertext, not directly readable credentials.

## Sign in and assign roles

Use the one-time OTP sent by the website. Team roles are Owner, Admin, and Member. Owners handle highest-level management and handoff, Admins manage members and team configuration, and Members use authorized resources. Follow the current UI permission messages.

Verify email and least-required role before inviting. Removing, leaving, or downgrading affects sync access. The only Owner should transfer responsibility before departure.

## Protect the master key

The master key encrypts database passwords, SSH private keys, and related secrets locally. Personal sync uploads ciphertext. A new device still needs the correct master key, and the website cannot recover one that is forgotten.

Back it up through a secure offline method. Never send it through chat, email, shared Notes, or reuse it as an account password.

## Resolve sync conflicts

Conflicts occur when both sides edit a record, or when one deletes while the other edits. Before choosing local or remote, compare device, time, target, and fields. Record both versions manually if necessary.

Resolution can overwrite one side. After network recovery, sync a small set first and avoid simultaneous bulk rename or delete on multiple devices.

## Manage team keys and rotation

An Owner or Admin initializes the team key. Shared team records use it, and protected copies are stored on member devices. Version mismatch, missing key state, or revoked permission can pause team sync.

Rotation re-encrypts team records in bulk. Ensure an administrator device is online, compatible, and on a stable network; avoid mass edits during rotation. Have members resync and sample important connections afterward.

## Operate teams securely

Review Owner, Admin, Member, and shared-resource scope regularly. On departure, remove team access and rotate team and remote-service credentials when risk warrants it. Removing Navop membership does not revoke database, SSH, or cloud accounts in those systems.

Do not claim that the website, sync service, or administrators can directly decrypt connection secrets. For decryption failures, check account, master key, team key, version, and record ownership without publishing the encrypted database.
