// Reproducible runtime-only import from the pinned local upstream checkout.
//
// Two modes:
//   node scripts/vendor-sayit.mjs            # import + apply mechanical patches (one-shot)
//   node scripts/vendor-sayit.mjs --verify   # audit the current vendor tree against PATCHES
//
// The vendor tree carries local modifications beyond the mechanical patches below
// (they were applied by hand and cannot be replayed mechanically). Each one is
// registered in MANUAL_PATCHES with a verify() probe so `--verify` can tell you
// whether the tree still matches the recorded customization state.
import fs from 'node:fs';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const source = path.join(root, 'refs/SayIt');
const dest = path.join(root, 'vendor/sayit');
const vendorFile = (...parts) => path.join(dest, ...parts);
const exists = (...parts) => fs.existsSync(vendorFile(...parts));
const readVendor = (...parts) => fs.readFileSync(vendorFile(...parts), 'utf8');

const IMPORT_MAP = [
  ['LICENSE', 'LICENSE'],
  ['client/src', 'frontend/src'],
  ['client/src-tauri/src', 'native/src'],
  ['client/src-tauri/resources', 'native/resources'],
  ['client/src-tauri/icons', 'native/icons'],
  ['client/src-tauri/Cargo.toml', 'native/Cargo.toml'],
  ['client/src-tauri/build.rs', 'native/build.rs'],
  ['client/src-tauri/tauri.conf.json', 'native/tauri.conf.json'],
  ['client/tsconfig.json', 'frontend/tsconfig.json'],
  ['client/tailwind.config.cjs', 'frontend/tailwind.config.cjs'],
  ['client/overlay.html', 'frontend/overlay.html'],
  ['client/tray-menu.html', 'frontend/tray-menu.html'],
];

// ---------------------------------------------------------------------------
// Mechanical patches: applied automatically right after import, replayable.
// Each entry has apply() (writes) and verify() (read-only probe).
// ---------------------------------------------------------------------------
const MECHANICAL_PATCHES = [
  {
    id: 'namespace-migration',
    describe: 'All .rs files: "com.sayit.app" → "app.soundbridge.windows/sayit" (isolated data dir).',
    apply() {
      for (const entry of fs.readdirSync(vendorFile('native/src'), { recursive: true })) {
        if (!entry.endsWith('.rs')) continue;
        const file = path.join(vendorFile('native/src'), entry);
        fs.writeFileSync(file, fs.readFileSync(file, 'utf8')
          .replaceAll('"com.sayit.app"', '"app.soundbridge.windows/sayit"'));
      }
    },
    verify() {
      for (const entry of fs.readdirSync(vendorFile('native/src'), { recursive: true })) {
        if (!entry.endsWith('.rs')) continue;
        if (readVendor('native/src', entry).includes('"com.sayit.app"')) {
          return `native/src/${entry} still references the standalone namespace`;
        }
      }
      return null;
    },
  },
  {
    id: 'handler-extraction',
    describe: 'Extract the full command table from upstream main.rs into handler.rs; delete main.rs (de-entrypoint).',
    apply() {
      const main = fs.readFileSync(path.join(source, 'client/src-tauri/src/main.rs'), 'utf8');
      const commands = main.split('.invoke_handler(tauri::generate_handler![')[1].split('])')[0];
      fs.writeFileSync(vendorFile('native/src/handler.rs'),
        `use crate::{commands, models, providers};\npub fn handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {\n tauri::generate_handler![${commands}]\n}\n`);
      fs.rmSync(vendorFile('native/src/main.rs'));
    },
    verify() {
      if (exists('native/src/main.rs')) return 'native/src/main.rs should have been removed';
      if (!exists('native/src/handler.rs')) return 'native/src/handler.rs is missing';
      if (!readVendor('native/src/handler.rs').startsWith('use crate::{commands, models, providers};')) {
        return 'handler.rs does not look like the generated command table';
      }
      return null;
    },
  },
];

// ---------------------------------------------------------------------------
// Manual patches: applied by hand after import; only verifiable, not replayable.
// When you modify the vendor tree by hand, add an entry here with a probe.
// ---------------------------------------------------------------------------
const MANUAL_PATCHES = [
  {
    id: 'remote-transport-embed',
    files: ['frontend/src/services/recorder/RemoteTransport.ts', 'frontend/src/services/recorder/RecorderOrchestrator.ts',
      'frontend/src/services/remoteCapture.ts', 'frontend/src/RemoteWorkspace.tsx'],
    describe: 'Direct remote PCM capture transport (poll/ack/reject) wired into the recorder pipeline.',
    verify() {
      const transport = readVendor('frontend/src/services/recorder/RemoteTransport.ts');
      if (!transport.includes('remote_voice_poll')) return 'RemoteTransport.ts lost the poll loop';
      // 2026-09-07 review fix: stop() racing startRemoteRecording must not double-stop.
      if (!transport.includes('!this.stopped')) return 'RemoteTransport.ts lost the stop-race guard';
      return null;
    },
  },
  {
    id: 'update-chain-removed',
    files: ['frontend/src/features/settings/ServerSection.tsx', 'frontend/src/components/Sidebar.tsx'],
    describe: 'Application updates are managed by VoiceHub: update UI entries and the features/update module are removed (native rejects download/install commands).',
    verify() {
      if (exists('frontend/src/features/update')) return 'features/update directory should not exist';
      if (readVendor('frontend/src/features/settings/ServerSection.tsx').includes('checkForUpdateNow')) {
        return 'ServerSection still triggers update checks';
      }
      if (readVendor('frontend/src/components/Sidebar.tsx').includes('hasPendingUpdate')) {
        return 'Sidebar still shows the pending-update highlight';
      }
      return null;
    },
  },
  {
    id: 'dead-listeners-removed',
    files: ['frontend/src/App.tsx', 'frontend/src/overlay/main.tsx', 'frontend/src/services/bridge.ts',
      'frontend/src/types/appApi.d.ts'],
    describe: 'Listeners for events no backend ever emits were removed: open-about (main.rs deleted), overlay-ping (never had an emitter upstream), ptt-toggle (never emitted).',
    verify() {
      if (readVendor('frontend/src/App.tsx').includes("listen('open-about'")) return 'App.tsx still listens to open-about';
      if (readVendor('frontend/src/overlay/main.tsx').includes("listen<number>('overlay-ping'")) return 'overlay/main.tsx still listens to overlay-ping';
      if (readVendor('frontend/src/services/bridge.ts').includes('function onPTTToggle')) return 'bridge.ts still registers onPTTToggle';
      return null;
    },
  },
  {
    id: 'assets-vendored',
    files: ['frontend/src/assets/voicehub.svg'],
    describe: 'VoiceHub icon vendored locally; components import ../assets/voicehub.svg instead of a five-level relative path out of the vendor tree.',
    verify() {
      if (!exists('frontend/src/assets/voicehub.svg')) return 'vendored icon missing';
      for (const file of ['frontend/src/components/TitleBar.tsx', 'frontend/src/components/WelcomeGuide.tsx', 'frontend/src/pages/About.tsx']) {
        if (readVendor(...file.split('/')).includes('../../../../../src/assets')) {
          return `${file} still reaches outside the vendor tree for the icon`;
        }
      }
      return null;
    },
  },
  {
    id: 'home-input-source-card',
    files: ['frontend/src/pages/Home.tsx'],
    describe: 'Workspace home shows a microphone-input card: remote takes priority when connected; otherwise the PTT key dictates with the configured system mic (remote is optional).',
    verify() {
      const home = readVendor('frontend/src/pages/Home.tsx');
      if (!home.includes('selectedMic')) return 'Home.tsx lost the selectedMic wiring';
      if (!home.includes('Mic className')) return 'Home.tsx lost the input-source card';
      return null;
    },
  },
];

function importUpstream() {
  if (exists('native/src/lib.rs')) {
    throw new Error('The adapted runtime already exists. Re-import into a separate directory and review the diff.');
  }
  for (const [from, to] of IMPORT_MAP) {
    const target = vendorFile(...to.split('/'));
    if (fs.existsSync(target)) continue;
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.cpSync(path.join(source, from), target, { recursive: true });
  }
  for (const patch of MECHANICAL_PATCHES) {
    patch.apply();
    console.log(`applied mechanical patch: ${patch.id}`);
  }
  console.log('Import complete. Apply MANUAL_PATCHES by hand, then run --verify to audit.');
}

function verify() {
  let failures = 0;
  for (const patch of [...MECHANICAL_PATCHES, ...MANUAL_PATCHES]) {
    try {
      const problem = patch.verify();
      if (problem) {
        failures += 1;
        console.log(`FAIL [${patch.id}] ${problem}`);
      } else {
        console.log(`ok   [${patch.id}]`);
      }
    } catch (error) {
      failures += 1;
      console.log(`FAIL [${patch.id}] probe crashed: ${error.message}`);
    }
  }
  if (failures > 0) {
    console.log(`\n${failures} patch(es) no longer match the vendor tree.`);
    process.exitCode = 1;
  } else {
    console.log('\nAll recorded patches match the current vendor tree.');
  }
}

if (process.argv.includes('--verify')) {
  verify();
} else {
  importUpstream();
}
