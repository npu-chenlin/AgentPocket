"""生成 AgentPocket 的应用图标与托盘图标。

不引第三方素材：图标是一个"口袋"形的几何标记，用基本图形画出来，
保证仓库里能重新生成，不依赖二进制素材的来源。
"""

from PIL import Image, ImageDraw

TEAL = (34, 138, 120)
TEAL_DARK = (26, 110, 96)
WHITE = (255, 255, 255)


def rounded(size: int, radius_ratio: float = 0.22) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle(
        [0, 0, size - 1, size - 1],
        radius=int(size * radius_ratio),
        fill=TEAL,
    )
    return img


def pocket(size: int, img: Image.Image) -> Image.Image:
    """口袋：上半开口、下半收拢的容器轮廓。"""
    d = ImageDraw.Draw(img)
    s = size
    # 口袋主体：圆角矩形，压住下边做出开口感
    left, right = s * 0.30, s * 0.70
    top, bottom = s * 0.34, s * 0.72
    width = max(2, int(s * 0.085))
    d.rounded_rectangle(
        [left, top, right, bottom],
        radius=int(s * 0.06),
        outline=WHITE,
        width=width,
    )
    # 袋口：一道比主体更宽的横线，视觉上区分"口"和"身"
    lip_y = top + s * 0.10
    d.line(
        [(left - s * 0.05, lip_y), (right + s * 0.05, lip_y)],
        fill=WHITE,
        width=width,
    )
    # 袋内的一道弧线，暗示里面装着东西
    d.arc(
        [left + s * 0.09, top + s * 0.14, right - s * 0.09, bottom - s * 0.05],
        start=0,
        end=180,
        fill=WHITE,
        width=max(1, int(s * 0.055)),
    )
    return img


def main() -> None:
    # 应用图标：512 方形，Linux/macOS 用
    app = pocket(512, rounded(512))
    app.save("resources/icon.png")

    # Windows 要 ico，electron-builder 从 256 方形起
    sizes = [16, 24, 32, 48, 64, 128, 256]
    ico_base = pocket(256, rounded(256))
    ico_base.save(
        "resources/icon.ico",
        sizes=[(s, s) for s in sizes],
    )

    # 托盘：深浅背景都要能看清，所以用不透明底 + 白标记
    tray = rounded(32, radius_ratio=0.2)
    pocket(32, tray)
    tray.save("resources/tray.png")
    tray.resize((64, 64), Image.LANCZOS).save("resources/tray@2x.png")

    # 托盘的模板图版本（macOS 菜单栏）：纯黑白，系统自动反色
    template = Image.new("RGBA", (32, 32), (0, 0, 0, 0))
    d = ImageDraw.Draw(template)
    left, right, top, bottom = 32 * 0.30, 32 * 0.70, 32 * 0.34, 32 * 0.72
    width = max(2, int(32 * 0.085))
    d.rounded_rectangle(
        [left, top, right, bottom], radius=int(32 * 0.06), outline=(0, 0, 0, 255), width=width
    )
    lip_y = top + 32 * 0.10
    d.line([(left - 32 * 0.05, lip_y), (right + 32 * 0.05, lip_y)], fill=(0, 0, 0, 255), width=width)
    d.arc(
        [left + 32 * 0.09, top + 32 * 0.14, right - 32 * 0.09, bottom - 32 * 0.05],
        start=0,
        end=180,
        fill=(0, 0, 0, 255),
        width=max(1, int(32 * 0.055)),
    )
    template.save("resources/trayTemplate.png")

    print("已生成: icon.png icon.ico tray.png tray@2x.png trayTemplate.png")


if __name__ == "__main__":
    main()