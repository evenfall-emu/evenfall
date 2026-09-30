import { emailLogo } from "./email-brand";

/** The logo is carried in the message; all other content remains live text. */
export function signInEmail(url: string) {
  // The URL comes from Better Auth. Escape it for both attributes and text,
  // without changing the original URL in the plain-text alternative.
  const link = url.replace(/[&<>"']/g, (character) => {
    return {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;",
    }[character]!;
  });
  return {
    subject: "Sign in to Evenfall Player",
    attachments: [
      {
        filename: "evenfall-logo.png",
        content: emailLogo,
        type: "image/png",
        disposition: "inline",
        contentId: "evenfall-logo",
      },
    ] satisfies EmailAttachment[],
    text: `Sign in to Evenfall Player

Use this link to sign in to your Evenfall Player account:

${url}

This link expires in 5 minutes and can be used once. Keep it private.
If you did not request this email, you can safely ignore it.

Evenfall Player`,
    html: `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Sign in to Evenfall Player</title>
</head>
<body style="margin:0;padding:0;background-color:#101c2a;color:#e3eaf0;font-family:Arial,Helvetica,sans-serif;">
  <div style="display:none;max-height:0;overflow:hidden;mso-hide:all;">Your Evenfall Player sign-in link. Valid for 5 minutes, one use only.</div>
  <table role="presentation" width="100%" cellspacing="0" cellpadding="0" border="0" style="background-color:#101c2a;font-family:Arial,Helvetica,sans-serif;">
    <tr><td align="center" style="padding:32px 16px;">
      <!--[if mso]><table role="presentation" width="560" cellspacing="0" cellpadding="0" border="0"><tr><td><![endif]-->
      <table role="presentation" width="100%" cellspacing="0" cellpadding="0" border="0" style="max-width:560px;">
        <tr><td style="padding:0 0 24px;">
          <img src="cid:evenfall-logo" width="260" height="64" alt="Evenfall" style="display:block;border:0;color:#fff0c8;font-size:24px;">
        </td></tr>
        <tr><td style="background-color:#192b3a;padding:32px 24px;border-top:4px solid #8cdbbe;">
          <h1 style="margin:0 0 20px;color:#fff0c8;font-size:26px;line-height:34px;">Sign in to Evenfall Player</h1>
          <p style="margin:0 0 24px;color:#e3eaf0;font-size:16px;line-height:26px;">Use the button below to sign in to your account and get back to your games.</p>
          <table role="presentation" cellspacing="0" cellpadding="0" border="0">
            <tr><td align="center" style="background-color:#8cdbbe;border:1px solid #8cdbbe;mso-padding-alt:16px 24px;">
              <a href="${link}" style="display:inline-block;padding:16px 20px;color:#101c2a;font-size:16px;line-height:24px;font-weight:bold;text-decoration:none;">Sign in to Evenfall Player</a>
            </td></tr>
          </table>
          <p style="margin:24px 0 0;color:#e3eaf0;font-size:14px;line-height:22px;">This link expires in <strong>5 minutes</strong> and can be used <strong>once</strong>. Keep it private.</p>
          <p style="margin:16px 0 0;color:#a9b8c7;font-size:14px;line-height:22px;">If you did not request this email, you can safely ignore it.</p>
        </td></tr>
        <tr><td style="padding:24px 0 0;">
          <p style="margin:0 0 8px;color:#a9b8c7;font-size:13px;line-height:21px;">Button not working? Copy and paste this link into your browser:</p>
          <table role="presentation" width="100%" cellspacing="0" cellpadding="0" border="0" style="table-layout:fixed;">
            <tr><td style="word-break:break-all;overflow-wrap:anywhere;"><a href="${link}" style="color:#8cdbbe;font-size:12px;line-height:20px;text-decoration:underline;word-break:break-all;">${link}</a></td></tr>
          </table>
          <p style="margin:24px 0 0;color:#a9b8c7;font-size:12px;line-height:20px;">Evenfall Player</p>
        </td></tr>
      </table>
      <!--[if mso]></td></tr></table><![endif]-->
    </td></tr>
  </table>
</body>
</html>`,
  };
}
