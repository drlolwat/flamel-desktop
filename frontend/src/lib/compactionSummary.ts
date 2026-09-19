export type CompactionSectionKey =
  | "current"
  | "goals"
  | "decisions"
  | "completed"
  | "active"
  | "remaining"
  | "next"
  | "risks"
  | "artifacts"
  | "summary";

export interface CompactionSection {
  key: CompactionSectionKey;
  title: string;
  body: string;
}

const HEADINGS: Array<[string, CompactionSectionKey, string]> = [
  ["Current session", "current", "At checkpoint"],
  ["Goals", "goals", "Goals"],
  ["Decisions landed", "decisions", "Decisions"],
  ["Completed work", "completed", "Completed"],
  ["Work in progress", "active", "In progress"],
  ["Remaining work", "remaining", "Remaining"],
  ["Next actions", "next", "Next actions"],
  ["Risks and blockers", "risks", "Risks and blockers"],
  ["Key artifacts", "artifacts", "Key artifacts"],
];

const HEADING_LOOKUP = new Map(
  HEADINGS.map(([source, key, title]) => [source.toLowerCase(), { key, title }]),
);
const HEADING_PATTERN = new RegExp(
  `^(${HEADINGS.map(([source]) => source.replaceAll(" ", "\\s+")).join("|")}):\\s*(.*)$`,
  "i",
);

export function parseCompactionSummary(summary: string): CompactionSection[] {
  const sections: CompactionSection[] = [];
  const preamble: string[] = [];
  let current: CompactionSection | null = null;

  for (const line of summary.trim().split(/\r?\n/)) {
    const match = line.match(HEADING_PATTERN);
    if (match) {
      const heading = HEADING_LOOKUP.get(match[1].toLowerCase().replace(/\s+/g, " "));
      if (!heading) continue;
      current = { ...heading, body: match[2].trim() };
      sections.push(current);
      continue;
    }
    if (current) {
      current.body = `${current.body}${current.body ? "\n" : ""}${line}`;
    } else {
      preamble.push(line);
    }
  }

  const preambleBody = preamble.join("\n").trim();
  if (preambleBody) {
    sections.unshift({ key: "summary", title: "Checkpoint", body: preambleBody });
  }

  const useful = sections
    .map((section) => ({ ...section, body: section.body.trim() }))
    .filter((section) => section.body && section.body.toLowerCase() !== "unknown");

  return useful.length > 0
    ? useful
    : [{ key: "summary", title: "Checkpoint", body: summary.trim() }];
}
