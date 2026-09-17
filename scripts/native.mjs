import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { delimiter, join } from 'node:path';

// Also works immediately after rustup installation, before reopening the shell.
const env = { ...process.env };
const pathKey = Object.keys(env).find((key) => key.toUpperCase() === 'PATH') ?? 'PATH';
env[pathKey] = [join(homedir(), '.cargo', 'bin'), env[pathKey]].filter(Boolean).join(delimiter);
const [mode, ...extra] = process.argv.slice(2);
const manifest = ['--manifest-path', 'src-tauri/Cargo.toml'];
const commands = mode === 'check' ? [
  ['cargo', ['fmt', ...manifest, '--check']],
  ['cargo', ['clippy', ...manifest, '--all-targets', '--locked', '--', '-D', 'warnings']],
  ['cargo', ['test', ...manifest, '--locked']],
] : [['tauri', [mode, ...extra]]];

for (const [command, args] of commands) {
  const result = spawnSync(command, args, { env, stdio: 'inherit', shell: process.platform === 'win32' });
  if (result.error) console.error(`无法运行 ${command}：${result.error.message}。请执行 pnpm run doctor 检查环境。`);
  if (result.status !== 0) process.exit(result.status ?? 1);
}
