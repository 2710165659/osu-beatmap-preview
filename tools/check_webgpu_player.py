from pathlib import Path

from playwright.sync_api import sync_playwright


def main() -> None:
    errors: list[str] = []
    requests: list[str] = []
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(
            headless=True,
            args=["--enable-unsafe-webgpu", "--enable-features=Vulkan"],
        )
        page = browser.new_page(viewport={"width": 1440, "height": 900})
        page.on("console", lambda message: errors.append(f"console {message.type}: {message.text}") if message.type == "error" else None)
        page.on("pageerror", lambda error: errors.append(f"pageerror: {error}"))
        page.on("request", lambda request: requests.append(request.url))
        page.goto("http://127.0.0.1:8787", wait_until="domcontentloaded")
        if not page.evaluate("navigator.gpu !== undefined"):
            raise RuntimeError("测试浏览器未启用 WebGPU")
        page.locator("#bid").fill("738063")
        page.locator("#load-form button").click()
        try:
            page.locator("#play-page").wait_for(state="visible", timeout=30_000)
        except Exception:
            print({"url": page.url, "load_log": page.locator("#load-log").text_content(), "play_log": page.locator("#play-log").text_content(), "status": page.locator("#status").text_content(), "errors": errors})
            raise
        page.wait_for_timeout(1_000)
        before = page.evaluate("""() => ({
          type: session.constructor.name,
          mode: session.mode(),
          status: document.querySelector('#status').textContent,
          audio: { readyState: audio.readyState, seekable: audio.seekable.length ? audio.seekable.end(0) : 0 },
        })""")
        page.locator("#play").click()
        page.wait_for_timeout(3_000)
        page.evaluate("""() => {
          seek.value = '30000';
          seek.dispatchEvent(new Event('input', { bubbles: true }));
        }""")
        page.wait_for_timeout(750)
        after = page.evaluate("""() => ({
          playing, position, audio: { currentTime: audio.currentTime, paused: audio.paused },
          log: document.querySelector('#play-log').textContent,
        })""")
        page.screenshot(path=Path("target/player-web/webgpu-player.png"), full_page=True)
        if before["type"] != "WebGpuSession":
            raise RuntimeError(f"未使用 WebGpuSession：{before['type']}")
        if before["audio"]["readyState"] != 4 or before["audio"]["seekable"] <= 0:
            raise RuntimeError(f"音频未就绪：{before['audio']}")
        if not 29_500 <= after["position"] <= 30_750 or after["audio"]["paused"]:
            raise RuntimeError(f"播放或拖动失败：{after}")
        if any("/api/frame" in request for request in requests):
            raise RuntimeError("检测到旧的逐帧 HTTP 请求")
        if errors:
            raise RuntimeError("; ".join(errors))
        print({"before": before, "after": after, "request_count": len(requests)})
        browser.close()


if __name__ == "__main__":
    main()
