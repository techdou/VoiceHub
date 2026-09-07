# Embedded SayIt

Source: https://github.com/crosswk/SayIt
Revision: fcb0cc2e9bd62143c861a685fc2fc82226f83c92 (0.1.9)
Copyright: Liu Qianglong and SayIt contributors, 2026.
License: GNU Affero General Public License version 3; see LICENSE.

VoiceHub includes and modifies this runtime. It uses the existing VoiceHub hardware layer
(formerly SoundBridge) and adds direct remote PCM capture, integrated navigation and
custom ASR configuration. Standalone SayIt update installation is disabled.
The embedded runtime uses its own data directory and does not modify the
standalone SayIt installation. Upstream server connection remains configurable;
the server application is not bundled in the desktop executable.

The combined application is subject to AGPL-3.0. Original SoundBridge modules
retain their GPL-3.0 notices. Dependencies retain their respective licenses.

## Local modification ledger

All local modifications are registered in `scripts/vendor-sayit.mjs`:
mechanical patches (namespace migration, handler.rs extraction) replay
automatically on import; manual patches (remote PCM transport, update-chain
removal, dead-listener cleanup, vendored assets) are hand-applied and probed
by `node scripts/vendor-sayit.mjs --verify`. When you modify this tree by
hand, add a patch entry with a verify() probe so the audit stays truthful.
