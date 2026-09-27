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
      // VoiceHub 自有命令必须在生成的表之外补注册（上游 main.rs 永远不会列它们）。
      const owned = '            // Custom GGUF model (VoiceHub-owned: register/read custom model paths).\n'
        + '            models::custom::custom_model_path,\n'
        + '            models::custom::register_custom_model,\n';
      const anchor = '            models::registry::list_available_models,';
      const table = commands.includes(anchor)
        ? commands.replace(anchor, owned + anchor)
        : commands + '\n' + owned;
      fs.writeFileSync(vendorFile('native/src/handler.rs'),
        `use crate::{commands, models, providers};\npub fn handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {\n tauri::generate_handler![${table}]\n}\n`);
      fs.rmSync(vendorFile('native/src/main.rs'), { force: true });
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
  {
    id: 'update-chain-removed',
    describe: 'Updates are managed by VoiceHub: delete the update-notification module and the features/update UI; strip the command registrations (native system.rs update commands stay, matching the 0.2.0 sync).',
    apply() {
      fs.rmSync(vendorFile('native/src/commands/update_notification.rs'), { force: true });
      fs.rmSync(vendorFile('frontend/src/features/update'), { recursive: true, force: true });
      fs.rmSync(vendorFile('frontend/src/update-notification'), { recursive: true, force: true });
      const stripLines = (file, markers) => {
        const target = vendorFile(file);
        if (!fs.existsSync(target)) return;
        const kept = fs.readFileSync(target, 'utf8')
          .split('\n')
          .filter((line) => !markers.some((marker) => line.includes(marker)));
        fs.writeFileSync(target, kept.join('\n'));
      };
      stripLines('native/src/commands/mod.rs', ['pub mod update_notification;']);
      stripLines('native/src/handler.rs', ['commands::update_notification::']);
    },
    verify() {
      if (exists('frontend/src/features/update')) return 'features/update directory should not exist';
      if (exists('frontend/src/update-notification')) return 'update-notification window entry should not exist';
      if (exists('native/src/commands/update_notification.rs')) return 'update_notification command module should not exist';
      if (readVendor('native/src/handler.rs').includes('update_notification')) {
        return 'handler.rs still registers update_notification commands';
      }
      if (readVendor('native/src/commands/mod.rs').includes('pub mod update_notification')) {
        return 'commands/mod.rs still declares the update_notification module';
      }
      if (readVendor('frontend/src/App.tsx').includes('features/update')) {
        return 'App.tsx still imports the features/update module';
      }
      if (readVendor('frontend/src/features/settings/ServerSection.tsx').includes('checkForUpdateNow')) {
        return 'ServerSection still triggers update checks';
      }
      if (readVendor('frontend/src/components/Sidebar.tsx').includes('hasPendingUpdate')) {
        return 'Sidebar still shows the pending-update highlight';
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
    id: 'markdown-result-card',
    files: [
      'frontend/src/overlay/markdown.tsx', 'frontend/src/overlay/__tests__/markdown.test.tsx',
      'frontend/src/overlay/Overlay.tsx', 'frontend/src/services/recorder/OverlayService.ts',
      'frontend/src/services/recorder/RecorderOrchestrator.ts', 'native/src/window/mod.rs',
    ],
    describe: 'Selection explain-card: when a selection-edit AI result lands on a non-editable target (browser/PDF reading), it renders in an overlay result card as basic Markdown (zero-dependency renderer, escaped React nodes) instead of being pasted; the markdown-output instruction is appended to the selection prompt only on that path. Editable targets keep the paste-replace behavior.',
    verify() {
      if (!exists('frontend/src/overlay/markdown.tsx')) return 'markdown renderer missing';
      if (!exists('frontend/src/overlay/__tests__/markdown.test.tsx')) return 'markdown renderer tests missing';
      const overlay = readVendor('frontend/src/overlay/Overlay.tsx');
      if (!overlay.includes("state === 'result'")) return 'Overlay.tsx lost the result card branch';
      if (!overlay.includes('resultMarkdown')) return 'Overlay.tsx lost the resultMarkdown payload field';
      const service = readVendor('frontend/src/services/recorder/OverlayService.ts');
      if (!service.includes('showMarkdownResult')) return 'OverlayService lost showMarkdownResult';
      const orchestrator = readVendor('frontend/src/services/recorder/RecorderOrchestrator.ts');
      if (!orchestrator.includes('explainCardEligible')) return 'orchestrator lost the explain-card routing';
      if (!orchestrator.includes('explain-card')) return 'orchestrator lost the markdown-format prompt injection';
      const window = readVendor('native/src/window/mod.rs');
      if (!window.includes('OverlayLayout::Result')) return 'native overlay lost the Result layout';
      return null;
    },
  },
  {
    id: 'voicehub-owned-extensions',
    files: [
      'native/Cargo.toml', 'native/build.rs',
      'native/src/models/custom.rs', 'native/src/models/gguf_asr.rs',
      'native/src/providers/asr_openai_compat.rs', 'native/src/providers/registry.rs',
      'native/src/providers/asr_groq.rs', 'native/src/storage/mod.rs', 'native/src/handler.rs',
      'frontend/src/services/audio.ts', 'frontend/src/services/recorder/__tests__/RemoteTransport.test.ts',
      'frontend/src/features/settings/CustomLocalModel.tsx', 'frontend/src/features/settings/VoiceEnginePage.tsx',
      'frontend/src/features/settings/asrProfileStore.ts', 'frontend/src/features/settings/asrProviderCatalog.ts',
      'frontend/src/services/transcription/CloudAPIProvider.ts', 'frontend/src/pages/History.tsx',
      'frontend/src/themes/voicehub.ts', 'frontend/tailwind.config.cjs',
    ],
    describe: 'VoiceHub-owned extensions inside the vendor tree (cross-review 2026-09-16 hardened after the 0.2.0 sync dropped several of them): custom GGUF model path end to end (native loader branch + settings card + engine page entry), OpenAI-compatible ASR end to end (provider + registry routes + catalog card + runtime sync of apiUrl/model + transcription extra + history re-run extra), remote-PCM mic interception in audio.ts, storage seeds (workMode=local, aiEnabled=false), voicehub theme registration, and the tailwind content glob covering vendor pages. Every file listed here must exist and carry its probe marker.',
    verify() {
      for (const f of this.files) {
        if (!exists(f)) return `${f} missing (import wiped an owned extension)`;
      }
      const read = (f) => readVendor(f);
      const cargo = read('native/Cargo.toml');
      if (!cargo.includes('name = "voicehub-sayit"')) return 'Cargo.toml lost the package identity';
      if (!cargo.includes('vulkan = ["transcribe-cpp/vulkan"]')) return 'Cargo.toml lost the vulkan feature gate';
      const build = read('native/build.rs');
      if (!build.includes('pub fn stage_transcribe_runtime_libs')) return 'build.rs lost the pub staging fn';
      if (/^[^\/]*tauri_build::build\(\)/m.test(build)) return 'build.rs re-embeds tauri resources (duplicate VERSION)';
      if (!read('native/src/handler.rs').includes('custom_model_path')) return 'handler.rs lost custom model commands';
      const registry = read('native/src/providers/registry.rs');
      if (!registry.includes('asr_openai_compat::transcribe')) return 'registry.rs lost the openai_compat ASR route';
      if (!read('native/src/storage/mod.rs').includes('select_custom_model')) return 'storage lost select_custom_model';
      if (!read('native/src/storage/mod.rs').includes('workMode')) return 'storage lost the local-mode seed';
      if (!read('native/src/models/gguf_asr.rs').includes('super::custom::ID')) return 'gguf_asr lost the custom-path model branch';
      const audio = read('frontend/src/services/audio.ts');
      if (!audio.includes('REMOTE_MIC_ID')) return 'audio.ts lost the remote-mic interception';
      const profileStore = read('frontend/src/features/settings/asrProfileStore.ts');
      // 0.2.2 起 openai_compat 由上游原生实现（baseUrl/protocol 体系）；自有部分是
      // 旧私有键 cloudAsr.apiUrl → 0.2.2 档案的一次性迁移兜底。
      if (!profileStore.includes("getSetting('cloudAsr.apiUrl'")) return 'asrProfileStore lost the legacy openai_compat migration';
      const catalog = read('frontend/src/features/settings/asrProviderCatalog.ts');
      if (!catalog.includes("id: 'openai_compat'")) return 'asr catalog lost the openai_compat card';
      const cloud = read('frontend/src/services/transcription/CloudAPIProvider.ts');
      // 0.2.2 起自定义端点字段名是 baseUrl（上游原生），0.2.0 私有名 api_url 已并入。
      if (!cloud.includes("getSetting('cloudAsr.baseUrl'")) return 'CloudAPIProvider lost the custom-ASR endpoint wiring';
      if (!read('frontend/src/pages/History.tsx').includes("getSetting('cloudAsr.baseUrl'")) return 'History re-run lost the custom-ASR endpoint';
      if (!read('frontend/src/features/settings/VoiceEnginePage.tsx').includes('CustomLocalModel')) return 'engine page lost the custom model entry';
      if (!read('frontend/src/themes/index.ts').includes('voicehub')) return 'themes index lost the voicehub registration';
      const tailwind = read('frontend/tailwind.config.cjs');
      if (!tailwind.includes('vendor/sayit/frontend/src/**/*.{ts,tsx}')) return 'tailwind content glob lost the vendor page scan';
      return null;
    },
  },
  {
    id: 'hotkey-hooks-toggle',
    files: ['native/src/lib.rs'],
    describe: 'Host-facing set_hotkey_hooks_enabled(): when the dictation tool is not the embedded engine the SayIt keyboard hook is stopped, so the default HF (right Alt) / PTT (right Ctrl) keys no longer wake embedded recording or swallow the host trigger-key recorder.',
    verify() {
      const code = readVendor('native/src/lib.rs');
      if (!code.includes('pub fn set_hotkey_hooks_enabled')) return 'lib.rs lost set_hotkey_hooks_enabled';
      if (!code.includes('manager.stop()')) return 'set_hotkey_hooks_enabled lost the stop branch';
      return null;
    },
  },
  {
    id: 'simulator-nav-removed',
    files: ['frontend/src/components/Sidebar.tsx'],
    describe: 'The simulator nav entry is removed from the remote workspace sidebar: the simulator page itself was cut from the hardware app, so the link 404s.',
    verify() {
      const code = readVendor('frontend/src/components/Sidebar.tsx');
      if (code.includes('/remote/simulator')) return 'Sidebar still links /remote/simulator';
      if (!code.includes('/remote/buttons')) return 'Sidebar lost the buttons nav (probe anchor)';
      return null;
    },
  },
  {
    id: 'registry-first-source-loop',
    files: ['native/src/models/registry.rs'],
    describe: 'Local-model completeness scan unrolls the outer loop that upstream broke out of unconditionally (only the first source was ever checked); behavior is identical, the never-loops clippy denial is gone.',
    verify() {
      const code = readVendor('native/src/models/registry.rs');
      if (!code.includes('model.sources.first()')) return 'registry.rs lost the first() unroll';
      if (code.includes('break; // 只检查第一个 source')) return 'registry.rs still has the never-looping outer for';
      return null;
    },
  },
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
    id: 'settings-brand-voicehub',
    files: ['frontend/src/i18n/locales/zh-CN.json', 'frontend/src/i18n/locales/en.json'],
    describe: 'Client-self references renamed SayIt → 声枢 (zh) / VoiceHub (en) across settings/about/feedback/diagnostics copy; server-mode and upstream-release-log references keep the SayIt name honestly.',
    verify() {
      const zh = readVendor('frontend/src/i18n/locales/zh-CN.json');
      if (zh.includes('欢迎使用 SayIt') || zh.includes('打开 SayIt')) return 'zh locale still greets as SayIt';
      const en = readVendor('frontend/src/i18n/locales/en.json');
      if (en.includes('Welcome to SayIt')) return 'en locale still greets as SayIt';
      return null;
    },
  },
  {
    id: 'about-credits-list',
    files: ['frontend/src/pages/About.tsx'],
    describe: 'Open-source credits list: SayIt (upstream), whisper.cpp/GGML, Silero VAD, vibe-flow, remote-mic-app (+windows), VB-CABLE — names, roles, licenses, links (verified via gh api).',
    verify() {
      const about = readVendor('frontend/src/pages/About.tsx');
      if (!about.includes('whisper.cpp')) return 'About.tsx credits lost whisper.cpp';
      if (!about.includes('silero-vad')) return 'About.tsx credits lost Silero VAD';
      return null;
    },
  },
  {
    id: 'ui-polish-2026-09',
    files: ['frontend/src/pages/Home.tsx', 'frontend/src/components/TitleBar.tsx', 'frontend/src/index.css',
      'frontend/src/features/settings/WorkModeSection.tsx', 'frontend/src/features/settings/CloudAPISection.tsx',
      'frontend/src/features/settings/AsrTestSection.tsx', 'frontend/src/pages/About.tsx'],
    describe: 'UI polish round: home gains the AI-cleanup status card (4th slot), tighter transcript rows with hover/arrow; titlebar AI switch spaced from window controls; font stacks unified to the voicehub theme order; engine page billing warning, stronger selected-mode card, test placeholder; settings nav first item renamed 通用/General; About credits grouped with license tags.',
    verify() {
      const home = readVendor('frontend/src/pages/Home.tsx');
      if (!home.includes('AI cleanup') && !home.includes('AI 整理')) return 'Home.tsx lost the AI card';
      if (!home.includes('Wand2')) return 'Home.tsx AI card icon missing';
      const titlebar = readVendor('frontend/src/components/TitleBar.tsx');
      if (!titlebar.includes('w-10 shrink-0') || !titlebar.includes("WebkitAppRegion: 'drag'")) {
        return 'TitleBar drag-gap between AI switch and window controls regressed';
      }
      const about = readVendor('frontend/src/pages/About.tsx');
      if (!about.includes('CREDIT_GROUPS')) return 'About.tsx credits lost grouping';
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
