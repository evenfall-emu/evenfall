import { describe, expect, it } from "vite-plus/test";
import { signInEmail } from "../src/sign-in-email";

describe("sign-in email", () => {
  it("escapes HTML without changing the plain-text verification URL", () => {
    const url = 'https://example.com/verify?token=test&next="<tag>\'"';
    const escaped =
      "https://example.com/verify?token=test&amp;next=&quot;&lt;tag&gt;&#39;&quot;";
    const { text, html } = signInEmail(url);
    expect(text).toContain(`\n\n${url}\n\n`);
    expect(html.match(/href="[^"]+"/g)).toEqual([
      `href="${escaped}"`,
      `href="${escaped}"`,
    ]);
    expect(html).toContain(`>${escaped}</a>`);
    expect(html).not.toContain(url);
  });

  it("keeps the security guidance readable without external assets", () => {
    const { text, html } = signInEmail("https://example.com/verify?token=test");
    for (const content of [text, html.replace(/<[^>]+>/g, "")]) {
      expect(content).toContain("Sign in to Evenfall Player");
      expect(content).toContain("expires in 5 minutes");
      expect(content).toContain("can be used once");
      expect(content).toContain("Keep it private.");
      expect(content).toContain("If you did not request this email");
      expect(content).not.toContain("ZiGBA");
    }
    expect(html).not.toMatch(/<(svg|script|link)\b|url\(|@font-face/i);
  });

  it("carries every image as an inline PNG rather than a remote dependency", () => {
    const { html, attachments } = signInEmail("https://example.com/verify");
    const sources = [...html.matchAll(/src="([^"]+)"/g)].map(
      (match) => match[1],
    );
    expect(sources).toEqual(["cid:evenfall-logo"]);
    expect(attachments).toHaveLength(1);
    const logo = attachments[0];
    expect(logo).toMatchObject({
      contentId: "evenfall-logo",
      filename: "evenfall-logo.png",
      disposition: "inline",
      type: "image/png",
    });
    const png = Buffer.from(logo.content, "base64");
    expect(png.subarray(0, 8)).toEqual(
      Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    );
    expect(png.readUInt32BE(16)).toBe(520);
    expect(png.readUInt32BE(20)).toBe(128);
    expect(html).toContain('alt="Evenfall"');
  });
});
