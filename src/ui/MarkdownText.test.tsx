import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { MarkdownText } from "./MarkdownText";
import { ALLOWED_ELEMENTS, linkKind } from "../lib/markdownPolicy";

const render = (text: string, onOpenFile?: (p: string) => void) =>
  renderToStaticMarkup(<MarkdownText text={text} onOpenFile={onOpenFile} />);

describe("MarkdownText", () => {
  it("keeps raw HTML as literal text — the model never renders markup", () => {
    const html = render('Done. <img src=x onerror="alert(1)"> and <b>bold</b> <script>1</script>');
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<b>");
    expect(html).not.toContain("<script");
    expect(html).toContain(
      "Done. &lt;img src=x onerror=&quot;alert(1)&quot;&gt; and &lt;b&gt;bold&lt;/b&gt;",
    );
    expect(ALLOWED_ELEMENTS).not.toContain("img");
    expect(ALLOWED_ELEMENTS).not.toContain("script");
  });
  it("renders a GFM table in a horizontally scrollable wrapper", () => {
    const html = render("| file | status |\n|---|---|\n| a.ts | ok |");
    expect(html).toContain('data-testid="md-table"');
    expect(html).toContain("overflow-x-auto");
    expect(html).toMatch(/<th[^>]*>file<\/th>/);
    expect(html).toMatch(/<td[^>]*>ok<\/td>/);
  });
  it("links the web, turns a relative path into an opener, and drops the rest", () => {
    const html = render(
      "[docs](https://example.com/x) and [src/a.ts](src/a.ts) and [x](javascript:alert(1)) and [m](mailto:a@b.c)",
      () => {},
    );
    expect(html).toMatch(
      /<a href="https:\/\/example\.com\/x" target="_blank" rel="noreferrer noopener"/,
    );
    expect(html).toContain('data-testid="md-path"');
    expect(html).not.toContain("javascript:");
    expect(html).not.toContain("mailto:");
    expect(html).toContain(">x<");
    expect(linkKind("https://a")).toBe("web");
    expect(linkKind("src/lib/a.ts")).toBe("path");
    expect(linkKind("//evil")).toBe("none");
    expect(linkKind("data:text/html,x")).toBe("none");
  });
  it("renders nested lists", () => {
    const html = render("- a\n  - b\n    - c\n- d");
    expect(html.replace(/\n/g, "")).toMatch(
      /<ul[^>]*><li[^>]*>a<ul[^>]*><li[^>]*>b<ul[^>]*><li[^>]*>c<\/li>/,
    );
  });
  it("renders a fence with its info string, highlighted and escaped", () => {
    const html = render('```ts\nconst a = "<b>"; // hi\n```');
    expect(html).toContain('data-lang="ts"');
    expect(html).toContain(">const</span>");
    expect(html).toContain("&quot;&lt;b&gt;&quot;");
    expect(html).not.toContain("<b>");
    expect(render("```\nplain\n```")).toContain('data-testid="code-fence"');
  });
  it("renders inline code, emphasis, strikethrough and task lists", () => {
    const html = render("use `npm test`, **bold**, ~~old~~\n\n- [x] done\n- [ ] todo");
    expect(html).toMatch(/<code[^>]*>npm test<\/code>/);
    expect(html).toContain("<strong>bold</strong>");
    expect(html).toContain("<del>old</del>");
    expect(html).toMatch(/<input type="checkbox"[^>]*checked=""\/> done/);
    expect(html).toMatch(/<input type="checkbox"(?:(?!checked)[^>])*\/> todo/);
  });
});
