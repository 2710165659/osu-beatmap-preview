MAX_SUPPORTED_DURATION_MS = 10 * 60 * 1000  # 支持渲染的最大谱面时长

MAX_AREA_HEIGHT_0_TO_1_MIN = 4000  # [0, 1) 分钟最大列高
MAX_AREA_HEIGHT_1_TO_2_MIN = 5500  # [1, 2) 分钟最大列高
MAX_AREA_HEIGHT_2_TO_3_MIN = 7000  # [2, 3) 分钟最大列高
MAX_AREA_HEIGHT_3_TO_4_MIN = 8500  # [3, 4) 分钟最大列高
MAX_AREA_HEIGHT_4_TO_5_MIN = 10000  # [4, 5) 分钟最大列高
MAX_AREA_HEIGHT_5_TO_6_MIN = 11500  # [5, 6) 分钟最大列高
FIXED_COLUMN_COUNT_6_TO_10_MIN = 30  # [6, 10) 分钟固定列数

PLAYFIELD_WIDTH = 512  # catch 横向游玩宽度
PLAYFIELD_HEIGHT = 384  # catch 逻辑游玩高度
STABLE_FRUIT_START_Y = -100  # stable/lazer 中水果出现时的逻辑 Y
STABLE_CATCHER_Y = 340  # stable/lazer 中水果抵达判定线的逻辑 Y
OBJECT_RADIUS = 64  # lazer/stable catch 逻辑半径

PAGE_MARGIN_X = 20  # 图片左右外边距
PAGE_MARGIN_Y = 20  # 图片上下外边距
LEFT_PANEL_WIDTH = 12  # 左侧时间轴栏宽度
COLUMN_WIDTH = 360  # 单列 playfield 可视宽度
COLUMN_GAP = 100  # 列间距
PLAYFIELD_SIDE_PADDING = 64  # 左右可见留白
TOP_BUFFER = 28  # 顶部留白，避免顶部物件被裁切
OBJECT_BOTTOM_PADDING = 12  # 不绘制 catcher 的列，底部给物件预留的安全空间
DRAW_CATCHER_EACH_COLUMN = False  # 是否每列都绘制盘子和小人

TIME_LABEL_FONT_SIZE = 29  # 时间标签字号
TIME_LABEL_NOTE_GAP = 10  # 时间标签与备注的间距
LEFT_PANEL_BACKGROUND = (112, 112, 112, 255)  # 左侧栏背景色
IMAGE_BACKGROUND = (0, 0, 0, 255)  # 整体背景色
PLAYFIELD_BACKGROUND = (7, 7, 7, 255)  # catch playfield 背景色
PLAYFIELD_BORDER = (34, 34, 34, 255)  # playfield 边框色
RULER_TEXT = (232, 232, 232, 255)  # 时间文字颜色
KIAI_TIME_LABEL_COLOR = (95, 221, 108, 255)  # kiai 段时间标签颜色
MEASURE_LINE = (87, 87, 87)  # (220,220,220,96) 预混合到 playfield 背景 (7,7,7)
BEAT_LINE = (62, 62, 62)  # (200,200,200,72) 预混合到 playfield 背景 (7,7,7)

FRUIT_HYPER_GLOW_ALPHA = 0.7  # hyper fruit 发光层透明度
FRUIT_HYPER_GLOW_SCALE = 1.2  # hyper fruit 发光层放大倍率
DROPLET_SCALE = 0.8  # droplet 相对 fruit 的缩放
TINY_DROPLET_SCALE = 0.4  # tiny droplet 相对 fruit 的缩放
BANANA_SCALE = 0.6  # 香蕉在判定时的最终缩放

CATCHER_BASE_SIZE = 106.75  # Catcher.BASE_SIZE
CATCHER_ALLOWED_RANGE = 0.8  # Catcher.ALLOWED_CATCH_RANGE
LEGACY_CATCHER_VISUAL_SCALE = 0.35  # LegacyCatcher 的 0.5 * 0.7
LEGACY_CATCHER_ORIGIN_Y = 16  # stable catcher 顶部原点偏移

DEFAULT_BEAT_LENGTH = 500.0  # 缺少红线时默认 120 BPM
DEFAULT_METER = 4  # 缺少红线时默认 4/4
RNG_SEED = 1337  # CatchBeatmapProcessor 的固定随机种子

BANANA_COLORS = (
    (255, 240, 0),
    (255, 192, 0),
    (214, 221, 28),
)

# GIF 相关配置（2x2 宫格布局，与 std 一致）
GIF_ROW_COUNT = 2           # GIF 行数
GIF_IMAGES_PER_ROW = 2      # GIF 每行格数
GIF_SEGMENT_COUNT = GIF_ROW_COUNT * GIF_IMAGES_PER_ROW  # 总格数 = 4
GIF_DURATION_MS = 5000     # 每段播放时长(ms)
GIF_FPS = 15                # GIF 帧率
GIF_LOOP = 0                # GIF 循环次数，0 表示无限循环
GIF_IMAGE_WIDTH = 512       # 每格宽度（与 osu! catch playfield 一致）
GIF_IMAGE_HEIGHT = 384      # 每格高度（与 osu! catch playfield 一致）
GIF_GRID_GAP = 20           # 格间距（与 std 一致）
GIF_LEFT_PANEL_WIDTH = 12   # 每格左侧灰色区域宽度（与 std 一致）
GIF_LEFT_PANEL_BACKGROUND = (112, 112, 112, 255)  # 每格左侧区域颜色
GIF_TIME_LABEL_FONT_SIZE = 30        # GIF 时间标签字号
GIF_TIME_LABEL_NOTE_FONT_SIZE = 22   # GIF PreviewTime 备注字号
GIF_TIME_LABEL_HEIGHT = 76           # GIF 时间标签预留高度
GIF_TIME_LABEL_TOP_GAP = 8           # 格与时间标签之间间距
GIF_TIME_LABEL_NOTE_TOP_GAP = 9      # 主时间与备注间距
GIF_TIME_LABEL_COLOR = (232, 232, 232, 255)
GIF_TIME_LABEL_NOTE_COLOR = (170, 170, 170, 255)
GIF_PREVIEW_TIME_LABEL_COLOR = (95, 221, 108, 255)
