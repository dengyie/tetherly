// Browser-tier flows: the SPA served at /next/ against a scripted mock
// daemon. These pin the full user path — load, token fragment handling,
// navigation, live event updates — that component-tier tests can only
// approximate.
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/next/");
  await expect(page.getByText("e2e-mac")).toBeVisible();
});

test("flow: loads, connects the stream and renders the overview", async ({ page }) => {
  await expect(page.getByText("E2E-WIN")).toBeVisible();
  await expect(page.getByText(/0\.2\.58/)).toBeVisible();
  // linkState goes live once the SSE stream delivered the snapshot
  await expect(page.locator("header .link")).toHaveText("", { timeout: 5000 });
});

test("flow: activity page renders the four feeds from the event stream", async ({ page }) => {
  await page.getByRole("button", { name: "动态" }).click();
  await expect(page.getByText("Your login code")).toBeVisible();
  await expect(page.getByText("hello from e2e")).toBeVisible();
  await expect(page.getByText("a page")).toBeVisible();
  await expect(page.getByText("待处理")).toBeVisible();
});

test("flow: navigation reaches every page without errors", async ({ page }) => {
  for (const name of ["设备", "传输", "收件箱", "布局", "设置", "诊断"]) {
    await page.getByRole("button", { name }).click();
    // Each page shows its <h1>; no crash, no blank main.
    await expect(page.locator("main h1")).toBeVisible();
  }
});

test("flow: pages render the event-fed sections", async ({ page }) => {
  await page.getByRole("button", { name: "布局" }).click();
  await expect(page.getByRole("group", { name: "右侧" })).toContainText("E2E-WIN");
  await expect(page.getByRole("switch", { name: "启用跨屏" })).toHaveAttribute("aria-checked", "true");
  await page.getByRole("button", { name: "传输" }).click();
  await expect(page.getByText("report.pdf")).toBeVisible();
  await expect(page.getByRole("progressbar").first()).toHaveAttribute("aria-valuenow", "62");
  await page.getByRole("button", { name: "收件箱" }).click();
  await expect(page.getByText("photo.jpg")).toBeVisible();
  await page.getByRole("button", { name: "设置" }).click();
  await expect(page.getByRole("switch", { name: "mDNS 广播" })).toHaveAttribute("aria-checked", "true");
  // ⌘K palette jumps between pages
  await page.keyboard.press("ControlOrMeta+k");
  await page.getByPlaceholder("输入命令或搜索…").fill("诊断");
  await page.keyboard.press("Enter");
  await expect(page.locator("main h1")).toHaveText("诊断");
});

test("flow: token fragment is stripped and stored, not kept in the URL", async ({ page }) => {
  // A hash-only navigation does not remount the SPA, so load a fresh page
  // with the fragment already in place.
  await page.goto("/next/#token=e2e-secret", { waitUntil: "commit" });
  await page.reload();
  await expect(page.getByText("e2e-mac")).toBeVisible();
  await expect
    .poll(() => Promise.resolve(new URL(page.url()).hash))
    .toBe("");
  const token = await page.evaluate(() => sessionStorage.getItem("tetherly_token"));
  expect(token).toBe("e2e-secret");
});
