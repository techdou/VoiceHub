// Reproducible runtime-only import from the pinned local upstream checkout.
import fs from 'node:fs';
import path from 'node:path';
const root = path.resolve(import.meta.dirname, '..');
const source = path.join(root, 'refs/SayIt');
const dest = path.join(root, 'vendor/sayit');
if (fs.existsSync(path.join(dest, 'native/src/lib.rs'))) {
  throw new Error('The adapted runtime already exists. Re-import into a separate directory and review the diff.');
}
for (const [from, to] of [
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
]) {
  const target = path.join(dest, to);
  if (fs.existsSync(target)) continue;
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.cpSync(path.join(source, from), target, { recursive: true });
}
// Mechanical namespace migration keeps the standalone application's data isolated.
for (const entry of fs.readdirSync(path.join(dest, 'native/src'), { recursive: true })) {
  if (!entry.endsWith('.rs')) continue;
  const file = path.join(dest, 'native/src', entry);
  fs.writeFileSync(file, fs.readFileSync(file, 'utf8').replaceAll('"com.sayit.app"', '"app.soundbridge.windows/sayit"'));
}
// Preserve the complete command table in an independently callable library.
const main = fs.readFileSync(path.join(source, 'client/src-tauri/src/main.rs'), 'utf8');
const commands = main.split('.invoke_handler(tauri::generate_handler![')[1].split('])')[0];
fs.writeFileSync(path.join(dest, 'native/src/handler.rs'),
  `use crate::{commands, models, providers};\npub fn handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {\n tauri::generate_handler![${commands}]\n}\n`);
fs.rmSync(path.join(dest, 'native/src/main.rs'));
