import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { PendingQuestion } from "@flamel/shared";
import { AskQuestionBlock } from "./AskQuestionBlock";

function renderQuestion(question: PendingQuestion) {
  return renderToStaticMarkup(
    <AskQuestionBlock question={question} onResolved={() => {}} />,
  );
}

describe("AskQuestionBlock", () => {
  it("renders an inline, accessible decision card with rich choices", () => {
    const html = renderQuestion({
      requestId: "question-1",
      question: "Which database should Flamel use?",
      choices: [
        {
          label: "SQLite (Recommended)",
          description: "Smallest operational footprint.",
        },
        {
          label: "Postgres",
          description: "Best when concurrent writers matter.",
        },
      ],
      allowFreeText: true,
    });

    expect(html).toContain("Input required");
    expect(html).toContain('aria-label="Answer choices"');
    expect(html).toContain("SQLite (Recommended)");
    expect(html).toContain("Smallest operational footprint.");
    expect(html).toContain("BEST FIT");
    expect(html).toContain("Custom answer");
    expect(html).toContain("Cancel");
    expect(html).not.toContain('role="dialog"');
    expect(html).not.toContain("aria-modal");
  });

  it("uses semantic checkboxes for multiple selections", () => {
    const html = renderQuestion({
      requestId: "question-2",
      question: "Which checks should run?",
      choices: ["Unit tests", "Typecheck"],
      multiSelect: true,
      allowFreeText: false,
    });

    expect(html).toContain("<fieldset");
    expect(html.match(/type="checkbox"/g)).toHaveLength(2);
    expect(html).toContain("Submit choices");
  });
});
