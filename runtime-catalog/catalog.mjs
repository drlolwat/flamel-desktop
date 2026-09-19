#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const skillSources = [1, 2, 3, 4].map((batch) =>
  JSON.parse(readFileSync(join(root, `runtime-catalog/skills-${batch}.json`), 'utf8')),
);
const skillsSource = {
  schemaVersion: 1,
  skills: skillSources.flatMap((source) => source.skills || []),
};
const personasSource = JSON.parse(readFileSync(join(root, 'runtime-catalog/personas.json'), 'utf8'));
const governance = JSON.parse(readFileSync(join(root, 'runtime-catalog/governance.json'), 'utf8'));
const metadataSources = ['marketplace-metadata.json', 'model-metadata.json'].map((name) =>
  JSON.parse(readFileSync(join(root, 'runtime-catalog', name), 'utf8')),
);
const evaluationSources = readdirSync(join(root, 'runtime-catalog'))
  .filter((name) => /^evaluations-.+\.json$/.test(name))
  .sort()
  .map((name) => JSON.parse(readFileSync(join(root, 'runtime-catalog', name), 'utf8')));
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const slug = /^[a-z0-9](?:[a-z0-9.-]{0,94}[a-z0-9])?$/;

function fail(message) {
  throw new Error(`Runtime Catalog: ${message}`);
}

function frontmatter(content, key) {
  const opening = content.startsWith('---\n') ? 4 : content.startsWith('---\r\n') ? 5 : 0;
  if (!opening) fail('persona source has no frontmatter');
  const end = content.indexOf('\n---', opening);
  if (end < 0) fail('persona source has unterminated frontmatter');
  for (const line of content.slice(opening, end).split(/\r?\n/)) {
    const split = line.indexOf(':');
    if (split < 0 || line.slice(0, split).trim() !== key) continue;
    return line.slice(split + 1).trim().replace(/^['"]|['"]$/g, '');
  }
  return '';
}

function walkMarkdown(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return walkMarkdown(path);
    return entry.isFile() && entry.name.endsWith('.md') ? [relative(root, path).replaceAll('\\', '/')] : [];
  });
}

function canonical(value) {
  return JSON.stringify(value);
}

function verifyAndBuild() {
  if (skillsSource.schemaVersion !== 1 || !Array.isArray(skillsSource.skills)) {
    fail('skills.json must use schemaVersion 1 and contain skills');
  }
  if (personasSource.schemaVersion !== 1 || !Array.isArray(personasSource.personas)
      || !Array.isArray(personasSource.sourceLedger)) {
    fail('personas.json must use schemaVersion 1 and contain personas and sourceLedger');
  }

  const identities = new Set();
  const hashes = new Set();
  const artifacts = [];
  const skillSlugs = new Set();
  for (const skill of skillsSource.skills) {
    const requiredStrings = ['slug', 'version', 'title', 'description', 'category', 'instructions'];
    if (skill.schemaVersion !== 1 || skill.kind !== 'skill'
        || requiredStrings.some((key) => typeof skill[key] !== 'string' || !skill[key].trim())
        || !slug.test(skill.slug) || !semver.test(skill.version)
        || !Array.isArray(skill.triggers) || skill.triggers.length === 0
        || !Array.isArray(skill.providerCompatibility)
        || skill.providerCompatibility.join(',') !== 'codex,claude'
        || typeof skill.defaultEnabled !== 'boolean') {
      fail(`invalid skill artifact ${skill.slug || '<unknown>'}`);
    }
    skillSlugs.add(skill.slug);
    addArtifact(artifacts, identities, hashes, skill.kind, skill.slug, skill.version, canonical(skill));
  }
  if (skillSlugs.size !== 39 || skillsSource.skills.length < skillSlugs.size) {
    fail(`expected 39 skills with immutable version history, found ${skillSlugs.size} and ${skillsSource.skills.length} versions`);
  }

  const reasoningEfforts = new Set(['low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
  for (const metadata of metadataSources) {
    if (metadata.schemaVersion !== 1 || !['marketplace_metadata', 'model_metadata'].includes(metadata.kind)
        || !slug.test(metadata.slug) || !semver.test(metadata.version)) {
      fail('invalid mutable metadata artifact');
    }
    if (metadata.kind === 'marketplace_metadata') {
      const entries = metadata.entries;
      const slugs = new Set();
      if (!Array.isArray(entries) || entries.length === 0 || entries.length > 64) {
        fail('marketplace metadata entries are invalid');
      }
      for (const entry of entries) {
        if (Object.keys(entry).sort().join(',') !== 'category,description,publisher,slug,status,title'
            || !slug.test(entry.slug) || slugs.has(entry.slug)
            || ['title', 'description', 'category', 'publisher'].some((key) =>
              typeof entry[key] !== 'string' || !entry[key].trim())
            || !['available', 'unavailable', 'deprecated'].includes(entry.status)) {
          fail(`invalid marketplace metadata entry ${entry.slug || '<unknown>'}`);
        }
        slugs.add(entry.slug);
      }
    } else {
      const models = metadata.models;
      const ids = new Set();
      if (!Array.isArray(models) || models.length === 0 || models.length > 64) {
        fail('model metadata entries are invalid');
      }
      for (const model of models) {
        if (Object.keys(model).sort().join(',') !== 'defaultReasoningEffort,id,label,provider,reasoningEfforts'
            || !slug.test(model.id) || ids.has(model.id)
            || typeof model.label !== 'string' || !model.label.trim()
            || !['codex', 'claude'].includes(model.provider)
            || !Array.isArray(model.reasoningEfforts)
            || new Set(model.reasoningEfforts).size !== model.reasoningEfforts.length
            || model.reasoningEfforts.some((effort) => !reasoningEfforts.has(effort))
            || (model.defaultReasoningEffort !== null
              && !model.reasoningEfforts.includes(model.defaultReasoningEffort))) {
          fail(`invalid model metadata entry ${model.id || '<unknown>'}`);
        }
        ids.add(model.id);
      }
    }
    addArtifact(
      artifacts,
      identities,
      hashes,
      metadata.kind,
      metadata.slug,
      metadata.version,
      canonical(metadata),
    );
  }

  const skillsByIdentity = new Map(skillsSource.skills.map((skill) => [`${skill.slug}@${skill.version}`, skill]));
  const evaluationIdentities = new Set();
  let evaluationCount = 0;
  for (const source of evaluationSources) {
    if (source.schemaVersion !== 1 || typeof source.batch !== 'string' || !source.batch
        || !Array.isArray(source.skillVersions) || !Array.isArray(source.cases)) {
      fail('invalid evaluation source');
    }
    const declared = [...source.skillVersions].sort();
    const actual = source.cases.map((entry) => `${entry.skill}@${entry.version}`).sort();
    if (declared.join(',') !== actual.join(',')) fail(`evaluation inventory drift for ${source.batch}`);
    for (const entry of source.cases) {
      const identity = `${entry.skill}@${entry.version}`;
      const skill = skillsByIdentity.get(identity);
      if (!skill || evaluationIdentities.has(identity)) fail(`invalid or duplicate evaluation ${identity}`);
      evaluationIdentities.add(identity);
      for (const section of ['positive', 'guardrail']) {
        const test = entry[section];
        if (!test || typeof test.prompt !== 'string' || test.prompt.length < 20
            || !Array.isArray(test.mustCover) || test.mustCover.length < 2
            || test.mustCover.some((term) => typeof term !== 'string'
              || !skill.instructions.toLowerCase().includes(term.toLowerCase()))) {
          fail(`${section} evaluation is not grounded in ${identity}`);
        }
      }
      if (!entry.nonTrigger || typeof entry.nonTrigger.prompt !== 'string'
          || entry.nonTrigger.prompt.length < 20 || entry.nonTrigger.expectedSkill === entry.skill
          || !skillSlugs.has(entry.nonTrigger.expectedSkill)) {
        fail(`non-trigger evaluation is invalid for ${identity}`);
      }
      evaluationCount += 1;
    }
  }
  const governed = [
    ...(governance.personaDerivedSkills || []),
    ...(governance.specialistSkills || []),
  ];
  if (governance.schemaVersion !== 1
      || governance.personaDerivedSkills?.length !== 33
      || governance.specialistSkills?.length !== 6
      || governance.explicitIntentSkills?.join(',') !== 'growth-strategy,tone-and-humor'
      || governance.nativeWrappers?.join(',') !== 'native-time,native-file-operations'
      || new Set(governed).size !== 39
      || governed.some((name) => !skillSlugs.has(name))) {
    fail('governance inventory must map 33 persona-derived and 6 specialist skills');
  }
  for (const skill of skillsSource.skills) {
    const explicit = governance.explicitIntentSkills.includes(skill.slug);
    if (skill.defaultEnabled === explicit) fail(`default activation drift for ${skill.slug}`);
  }

  const personaSlugs = new Set();
  for (const persona of personasSource.personas) {
    if (!persona || typeof persona.source !== 'string' || !persona.source.startsWith('agents/')
        || !slug.test(persona.slug) || !semver.test(persona.version)
        || typeof persona.description !== 'string' || !persona.description.trim()) {
      fail('invalid persona source entry');
    }
    const markdown = readFileSync(join(root, persona.source), 'utf8');
    if (frontmatter(markdown, 'name') !== persona.slug
        || frontmatter(markdown, 'version') !== persona.version
        || frontmatter(markdown, 'description') !== persona.description) {
      fail(`persona index drift for ${persona.source}`);
    }
    if (personaSlugs.has(persona.slug)) fail(`duplicate persona slug ${persona.slug}`);
    personaSlugs.add(persona.slug);
    const artifact = {
      schemaVersion: 1,
      kind: 'persona',
      slug: persona.slug,
      version: persona.version,
      description: persona.description,
      content: markdown,
    };
    addArtifact(artifacts, identities, hashes, 'persona', persona.slug, persona.version, canonical(artifact));
  }
  if (personaSlugs.size !== 24) fail(`expected 24 retained personas, found ${personaSlugs.size}`);

  const dispositions = new Map([
    ['retained_persona', 24],
    ['extracted_skill', 16],
    ['native_wrapper', 2],
  ]);
  const ledgerSources = new Set();
  for (const entry of personasSource.sourceLedger) {
    if (!entry || typeof entry.source !== 'string' || !entry.source.startsWith('agents/')
        || typeof entry.owner !== 'string' || !entry.owner
        || !dispositions.has(entry.disposition)) {
      fail('invalid source ledger entry');
    }
    if (ledgerSources.has(entry.source)) fail(`duplicate ledger source ${entry.source}`);
    ledgerSources.add(entry.source);
    dispositions.set(entry.disposition, dispositions.get(entry.disposition) - 1);
  }
  if (personasSource.sourceLedger.length !== 42
      || [...dispositions.values()].some((remaining) => remaining !== 0)) {
    fail('source ledger must contain 24 retained, 16 extracted, and 2 native wrappers');
  }

  const generalSources = walkMarkdown(join(root, 'agents')).filter((path) =>
    !['agents/README.md', 'agents/PERSONA-SKILL-MATRIX.md', 'agents/PLATFORM-GUIDELINES.md'].includes(path)
    && !path.startsWith('agents/project-skills/')
  );
  if (generalSources.length !== 42
      || generalSources.some((path) => !ledgerSources.has(path))
      || [...ledgerSources].some((path) => !generalSources.includes(path))) {
    fail('the general persona inventory and 42-source ledger differ');
  }
  if (artifacts.length > 128) fail('catalog exceeds the artifact count limit');
  return {
    artifacts: artifacts.sort((left, right) =>
      `${left.kind}:${left.slug}:${left.version}`.localeCompare(
        `${right.kind}:${right.slug}:${right.version}`,
      )),
    evaluationCount,
  };
}

function addArtifact(artifacts, identities, hashes, kind, artifactSlug, version, content) {
  const identity = `${kind}:${artifactSlug}:${version}`;
  if (identities.has(identity)) fail(`duplicate artifact identity ${identity}`);
  identities.add(identity);
  const bytes = Buffer.byteLength(content);
  if (!bytes || bytes > 128 * 1024) fail(`artifact ${identity} exceeds its byte limit`);
  const hash = createHash('sha256').update(content).digest('hex');
  if (hashes.has(hash)) fail(`duplicate artifact content for ${identity}`);
  hashes.add(hash);
  artifacts.push({ kind, slug: artifactSlug, version, content });
}

const { artifacts, evaluationCount } = verifyAndBuild();
const command = process.argv[2] || 'verify';
if (command === 'verify') {
  const bytes = artifacts.reduce((total, artifact) => total + Buffer.byteLength(artifact.content), 0);
  process.stdout.write(JSON.stringify({
    artifacts: artifacts.length,
    skills: artifacts.filter((artifact) => artifact.kind === 'skill').length,
    personas: artifacts.filter((artifact) => artifact.kind === 'persona').length,
    sourceLedger: personasSource.sourceLedger.length,
    evaluations: evaluationCount,
    bytes,
  }) + '\n');
} else if (command === 'payload') {
  const minRuntime = process.env.FLAMEL_RUNTIME_CATALOG_MIN_RUNTIME || '0.1.0';
  const maxRuntime = process.env.FLAMEL_RUNTIME_CATALOG_MAX_RUNTIME || '0.1.0';
  if (!semver.test(minRuntime) || !semver.test(maxRuntime)) fail('runtime bounds must be semver');
  process.stdout.write(JSON.stringify({
    reason: process.env.FLAMEL_RUNTIME_CATALOG_REASON || 'Publish reviewed Runtime Catalog candidate',
    minRuntime,
    maxRuntime,
    revocations: [],
    artifacts,
  }) + '\n');
} else {
  fail('expected verify or payload');
}
