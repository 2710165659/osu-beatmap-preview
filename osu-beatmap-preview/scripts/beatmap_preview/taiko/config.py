MAX_SUPPORTED_DURATION_MS = 10 * 60 * 1000  # 支持渲染的最大谱面时长

BASE_ROW_WIDTH_0_TO_1_MIN = 5200  # [0, 1) 分钟单行基础宽度；实际宽度 = 基础宽度 * BPM 宽度倍率
BASE_ROW_WIDTH_1_TO_2_MIN = 6400  # [1, 2) 分钟单行基础宽度
BASE_ROW_WIDTH_2_TO_3_MIN = 7600  # [2, 3) 分钟单行基础宽度
BASE_ROW_WIDTH_3_TO_4_MIN = 8800  # [3, 4) 分钟单行基础宽度
BASE_ROW_WIDTH_4_TO_5_MIN = 10000  # [4, 5) 分钟单行基础宽度
BASE_ROW_WIDTH_5_TO_6_MIN = 11200  # [5, 6) 分钟单行基础宽度
BASE_ROW_WIDTH_6_TO_10_MIN = 12800  # [6, 10) 分钟单行基础宽度

ROW_WIDTH_BPM_0_TO_180 = 1.0  # [0, 180) BPM 单行宽度倍率；SPACING_BPM 不为 0 时不生效
ROW_WIDTH_BPM_180_TO_240 = 1.15  # [180, 240) BPM 单行宽度倍率；SPACING_BPM 不为 0 时不生效
ROW_WIDTH_BPM_240_TO_300 = 1.3  # [240, 300) BPM 单行宽度倍率；SPACING_BPM 不为 0 时不生效
ROW_WIDTH_BPM_300_PLUS = 1.45  # [300, +inf) BPM 单行宽度倍率；SPACING_BPM 不为 0 时不生效

DRAW_DRUM_EACH_ROW = False  # 是否每行开头都绘制鼓
ROW_GAP = 37  # 行与行之间间距
ROW_HEIGHT = 160  # 单行高度
SPACING_BPM = 0.0  # 按多少 BPM 绘制横向间距；0 表示按游戏内 BPM/SV 效果绘制

PIXELS_PER_SCROLL_MULTIPLIER_MS = 0.14  # taiko 横向滚动基础像素系数
SCROLL_LENGTH_RATIO = 1.6  # taiko 游戏内实际 scrollLength 相对 inLength 的固定倍率
DEFAULT_BEAT_LENGTH = 500.0  # 缺少红线时默认 beatLength（120 BPM）
DEFAULT_METER = 4  # 缺少红线时默认拍号

PAGE_MARGIN_X = 16  # 图片左右外边距
PAGE_MARGIN_Y = 16  # 图片上下外边距
ROW_INNER_PADDING_X = 67  # 行内左右预留宽度
LABEL_RIGHT_PADDING = 3  # 时间标签右侧安全边距
MIN_BEAT_LINE_SPACING = 400  # 非小节拍线的最小像素间距
TIME_LABEL_FONT_SIZE = 48  # 时间标签字号
TIME_LABEL_NOTE_FONT_SIZE = 35  # 时间标签备注字号
BPM_FONT_SIZE = 45  # BPM 指示字号
TIME_LABEL_TOP_GAP = 0  # 行底部与时间标签之间的间距
TIME_LABEL_NOTE_TOP_GAP = 11  # 主时间与备注之间的间距
BPM_TOP_GAP = 11  # 备注与 BPM 之间的间距
TIME_LABEL_HEIGHT = 170  # 时间标签预留高度，包含主时间、备注与 BPM

IMAGE_BACKGROUND = (0, 0, 0, 255)  # 整体背景色
CENTRE_NOTE_COLOR = (235, 69, 44)  # don note 颜色
RIM_NOTE_COLOR = (67, 142, 172)  # kat note 颜色
ROLL_COLOR = (232, 198, 61)  # drumroll 颜色
SWELL_COLOR = (255, 188, 64)  # swell 颜色
MEASURE_LINE_COLOR = (255, 255, 255, 170)  # 小节线颜色
BEAT_LINE_COLOR = (220, 220, 220, 96)  # 普通拍线颜色
RULER_TEXT_COLOR = (232, 232, 232, 255)  # 时间标签颜色
ACCENT_LABEL_COLOR = (95, 221, 108, 255)  # kiai、BPM 等强调标签颜色

NORMAL_NOTE_SIZE_RATIO = 0.475  # 普通 note 直径占行高的比例
BIG_NOTE_SCALE = 1 / 0.65  # 大 note 相对普通 note 的放大倍率
SPAN_BODY_HEIGHT_RATIO = 0.72  # drumroll 身体高度占头部直径比例
SWELL_BODY_HEIGHT_RATIO = 0.8  # swell 身体高度占头部直径比例
