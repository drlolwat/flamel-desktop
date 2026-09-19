import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { TypedRevealMarkdown } from "./TypedReveal";

describe("TypedRevealMarkdown", () => {
  it("hides HTML comments from assistant prose", () => {
    const html = renderToStaticMarkup(
      <TypedRevealMarkdown text={"Before\n\n<!-- internal marker -->\n\nAfter"} />,
    );

    expect(html).toContain("Before");
    expect(html).toContain("After");
    expect(html).not.toContain("internal marker");
  });

  it("preserves HTML comments shown as code", () => {
    const html = renderToStaticMarkup(
      <TypedRevealMarkdown text={'`<!-- inline -->`\n\n```html\n<!-- fenced -->\n```'} />,
    );

    expect(html).toContain("inline");
    expect(html).toContain("fenced");
  });
});
