#!/usr/bin/env node
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { basename, extname, join, resolve } from 'node:path';
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';

const args = process.argv.slice(2);
const getArgValue = (name) => {
  const prefix = `${name}=`;
  const inline = args.find((arg) => arg.startsWith(prefix));
  if (inline) return inline.slice(prefix.length);
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};

const distDir = resolve(
  process.cwd(),
  getArgValue('--dist') ?? process.env.FLAMEL_PERF_DIST ?? process.env.VITE_OUT_DIR ?? 'dist',
);
const configPath = resolve(
  process.cwd(),
  getArgValue('--config') ?? process.env.FLAMEL_PERF_BUDGETS ?? 'performance-budgets.json',
);
const strictTargets = process.env.FLAMEL_PERF_STRICT === '1' || args.includes('--strict');
const jsonOutput = args.includes('--json');

const fail = (message) => {
  console.error(message);
  process.exit(1);
};

if (!existsSync(configPath)) {
  fail(`Performance budget config not found: ${configPath}`);
}

if (!existsSync(distDir)) {
  fail(`Build output not found: ${distDir}. Run a production build before checking budgets.`);
}

const config = JSON.parse(readFileSync(configPath, 'utf8'));

const collectFiles = (dir) => {
  const entries = readdirSync(dir, { withFileTypes: true });
  return entries.flatMap((entry) => {
    const fullPath = join(dir, entry.name);
    if (entry.isDirectory()) return collectFiles(fullPath);
    if (!entry.isFile()) return [];
    return [fullPath];
  });
};

const toKiB = (bytes) => bytes / 1024;
const round = (value) => Number(value.toFixed(1));
const sum = (values) => values.reduce((total, value) => total + value, 0);
const fileSize = (file) => statSync(file).size;
const gzipSize = (file) => gzipSync(readFileSync(file), { level: 9 }).length;
const brotliSize = (file) => brotliCompressSync(readFileSync(file), {
  params: { [constants.BROTLI_PARAM_QUALITY]: 11 },
}).length;

const summarizeFiles = (files) => ({
  count: files.length,
  rawKiB: round(toKiB(sum(files.map(fileSize)))),
  gzipKiB: round(toKiB(sum(files.map(gzipSize)))),
  brotliKiB: round(toKiB(sum(files.map(brotliSize)))),
});

const allFiles = collectFiles(distDir);
const jsFiles = allFiles.filter((file) => extname(file) === '.js');
const cssFiles = allFiles.filter((file) => extname(file) === '.css');
const indexHtmlPath = join(distDir, 'index.html');
const indexHtml = existsSync(indexHtmlPath) ? readFileSync(indexHtmlPath, 'utf8') : '';
const initialAssetFiles = [...indexHtml.matchAll(/(?:src|href)=["']([^"']+)["']/g)]
  .map((match) => match[1].split(/[?#]/, 1)[0])
  .filter((assetPath) => assetPath.endsWith('.js') || assetPath.endsWith('.css'))
  .map((assetPath) => resolve(distDir, assetPath.replace(/^\/+/, '')))
  .filter((assetPath) => existsSync(assetPath));
const initialJsFiles = [...new Set(initialAssetFiles.filter((file) => extname(file) === '.js'))];
const initialCssFiles = [...new Set(initialAssetFiles.filter((file) => extname(file) === '.css'))];
const entryCandidates = initialJsFiles.length
  ? initialJsFiles
  : jsFiles.filter((file) => /^index(?:-[\w-]+)?\.js$/.test(basename(file)));
const mainEntryFile = entryCandidates.sort((a, b) => fileSize(b) - fileSize(a))[0];

if (!mainEntryFile) {
  fail('Unable to find a Vite entry chunk matching index*.js in the build output.');
}

const summarizeSingleFile = (file) => ({
  path: file.replace(`${distDir}/`, ''),
  rawKiB: round(toKiB(fileSize(file))),
  gzipKiB: round(toKiB(gzipSize(file))),
  brotliKiB: round(toKiB(brotliSize(file))),
});

const metrics = {
  assetCount: allFiles.length,
  js: summarizeFiles(jsFiles),
  css: summarizeFiles(cssFiles),
  initialJs: summarizeFiles(initialJsFiles),
  initialCss: summarizeFiles(initialCssFiles),
  mainEntryJs: summarizeSingleFile(mainEntryFile),
};

const getMetric = (path) => path.split('.').reduce((value, part) => value?.[part], metrics);
const formatValue = (value, unit) => {
  const display = typeof value === 'number' ? value.toFixed(Number.isInteger(value) ? 0 : 1) : value;
  return !unit || unit === 'count' ? display : `${display} ${unit}`;
};

const rows = [];
const failures = [];
const targetMisses = [];

for (const budget of config.assetBudgets ?? []) {
  const actual = getMetric(budget.metric);
  if (typeof actual !== 'number') {
    failures.push(`${budget.id}: missing metric ${budget.metric}`);
    continue;
  }

  const overLimit = typeof budget.limit === 'number' && actual > budget.limit;
  const overTarget = typeof budget.target === 'number' && actual > budget.target;
  const status = overLimit ? 'fail' : overTarget ? 'warn' : 'pass';

  rows.push({
    id: budget.id,
    label: budget.label,
    actual,
    target: budget.target,
    limit: budget.limit,
    unit: budget.unit,
    status,
  });

  if (overLimit) {
    failures.push(`${budget.label}: ${formatValue(actual, budget.unit)} exceeds limit ${formatValue(budget.limit, budget.unit)}`);
  } else if (overTarget) {
    targetMisses.push(`${budget.label}: ${formatValue(actual, budget.unit)} is above target ${formatValue(budget.target, budget.unit)}`);
  }
}

if (strictTargets) {
  failures.push(...targetMisses.map((message) => `${message} (strict target)`));
}

if (jsonOutput) {
  console.log(JSON.stringify({ distDir, metrics, rows, failures, targetMisses, strictTargets }, null, 2));
} else {
  console.log(`Performance budget check: ${distDir}`);
  console.log(`Main entry: ${metrics.mainEntryJs.path}`);
  console.log('');
  for (const row of rows) {
    const marker = row.status === 'pass' ? 'PASS' : row.status === 'warn' ? 'WARN' : 'FAIL';
    console.log(`${marker} ${row.label}: ${formatValue(row.actual, row.unit)} target ${formatValue(row.target, row.unit)} limit ${formatValue(row.limit, row.unit)}`);
  }
  if (config.runtimeBudgets?.length) {
    console.log('');
    console.log(`Runtime budgets documented: ${config.runtimeBudgets.length}. Use profiling traces or instrumentation to enforce them.`);
  }
  if (targetMisses.length && !strictTargets) {
    console.log('');
    console.log('Targets missed without failing this run:');
    for (const miss of targetMisses) console.log(`- ${miss}`);
  }
}

if (failures.length) {
  if (!jsonOutput) {
    console.error('');
    console.error('Performance budget failures:');
    for (const failure of failures) console.error(`- ${failure}`);
  }
  process.exit(1);
}
