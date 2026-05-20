MAX_SUPPORTED_DURATION_MS = 10 * 60 * 1000  # 支持渲染的最大谱面时长

BASE_ROW_WIDTH_0_TO_1_MIN = 2600  # [0, 1) 分钟单行基础宽度；实际宽度 = 基础宽度 * BPM 宽度倍率
BASE_ROW_WIDTH_1_TO_2_MIN = 3200  # [1, 2) 分钟单行基础宽度
BASE_ROW_WIDTH_2_TO_3_MIN = 3800  # [2, 3) 分钟单行基础宽度
BASE_ROW_WIDTH_3_TO_4_MIN = 4400  # [3, 4) 分钟单行基础宽度
BASE_ROW_WIDTH_4_TO_5_MIN = 5000  # [4, 5) 分钟单行基础宽度
BASE_ROW_WIDTH_5_TO_6_MIN = 5600  # [5, 6) 分钟单行基础宽度
BASE_ROW_WIDTH_6_TO_10_MIN = 6400  # [6, 10) 分钟单行基础宽度

ROW_WIDTH_BPM_0_TO_180 = 1.0  # [0, 180) BPM 单行宽度倍率；SPACING_BPM 不为 0 时不生效
ROW_WIDTH_BPM_180_TO_240 = 1.15  # [180, 240) BPM 单行宽度倍率；SPACING_BPM 不为 0 时不生效
ROW_WIDTH_BPM_240_TO_300 = 1.3  # [240, 300) BPM 单行宽度倍率；SPACING_BPM 不为 0 时不生效
ROW_WIDTH_BPM_300_PLUS = 1.45  # [300, +inf) BPM 单行宽度倍率；SPACING_BPM 不为 0 时不生效

DRAW_DRUM_EACH_ROW = False  # 是否每行开头都绘制鼓
ROW_GAP = 80  # 行与行之间间距
ROW_HEIGHT = 80  # 单行高度
SPACING_BPM = 0.0  # 按多少 BPM 绘制横向间距；0 表示按游戏内 BPM/SV 效果绘制

PIXELS_PER_SCROLL_MULTIPLIER_MS = 0.07  # taiko 横向滚动基础像素系数
SCROLL_LENGTH_RATIO = 1.6  # taiko 游戏内实际 scrollLength 相对 inLength 的固定倍率
DEFAULT_BEAT_LENGTH = 500.0  # 缺少红线时默认 beatLength（120 BPM）
DEFAULT_METER = 4  # 缺少红线时默认拍号

PAGE_MARGIN_X = 8  # 图片左右外边距
PAGE_MARGIN_Y = 8  # 图片上下外边距
ROW_INNER_PADDING_X = 33  # 行内左右预留宽度
LABEL_RIGHT_PADDING = 1  # 时间标签右侧安全边距
MIN_BEAT_LINE_SPACING = 200  # 非小节拍线的最小像素间距
TIME_LABEL_FONT_SIZE = 24  # 时间标签字号
TIME_LABEL_NOTE_FONT_SIZE = 17  # 时间标签备注字号
BPM_FONT_SIZE = 22  # BPM 指示字号
TIME_LABEL_TOP_GAP = 0  # 行底部与时间标签之间的间距
TIME_LABEL_NOTE_TOP_GAP = 5  # 主时间与备注之间的间距
BPM_TOP_GAP = 5  # 备注与 BPM 之间的间距
SV_TEXT_COLOR = (255, 217, 102, 255)  # SV 指示文字颜色
SV_TEXT_FONT_SIZE = 15  # SV 指示字号
SV_TOP_GAP = 0  # SV 文字与行顶部之间的间距

IMAGE_BACKGROUND = (0, 0, 0, 255)  # 整体背景色
CENTRE_NOTE_COLOR = (235, 69, 44)  # don note 颜色
RIM_NOTE_COLOR = (67, 142, 172)  # kat note 颜色
ROLL_COLOR = (232, 198, 61)  # drumroll 颜色
SWELL_COLOR = (82, 204, 180)  # swell/转盘填充颜色
MEASURE_LINE_COLOR = (255, 255, 255, 170)  # 小节线颜色
BEAT_LINE_COLOR = (83, 83, 83)  # (220,220,220,96) 预混合到黑色行背景
RULER_TEXT_COLOR = (232, 232, 232, 255)  # 时间标签颜色
ACCENT_LABEL_COLOR = (95, 221, 108, 255)  # kiai、BPM 等强调标签颜色

NORMAL_NOTE_SIZE_RATIO = 0.475  # 普通 note 直径占行高的比例
BIG_NOTE_SCALE = 1 / 0.65  # 大 note 相对普通 note 的放大倍率
SPAN_BODY_HEIGHT_RATIO = 0.72  # drumroll 身体高度占头部直径比例
SWELL_BODY_HEIGHT_RATIO = 0.8  # swell 身体高度占头部直径比例

# GIF 相关配置
GIF_SEGMENT_COUNT = 4  # GIF 横向段数
GIF_DURATION_MS = 5000  # 每段播放时长(ms)
GIF_FPS = 15  # GIF 帧率
GIF_LOOP = 0  # GIF 循环次数，0 表示无限循环
GIF_ROW_HEIGHT = 80  # GIF 单行高度
GIF_ROW_GAP = 60  # GIF 行间距
GIF_TIME_LABEL_FONT_SIZE = 20  # GIF 时间标签字号
GIF_TIME_LABEL_NOTE_FONT_SIZE = 14  # GIF PreviewTime 备注字号
GIF_TIME_LABEL_COLOR = (232, 232, 232, 255)  # GIF 普通时间标签颜色
GIF_TIME_LABEL_NOTE_COLOR = (170, 170, 170, 255)  # GIF 普通备注标签颜色
GIF_PREVIEW_TIME_LABEL_COLOR = (95, 221, 108, 255)  # GIF PreviewTime 标签颜色
GIF_JUDGEMENT_LINE_COLOR = (255, 255, 255, 200)  # GIF 判定线颜色
# GIF 横向几何按游戏内 TaikoPlayfield 的本地坐标等比缩小：
# BASE_HEIGHT=200, INPUT_DRUM_WIDTH=180, hit_target_padding=76（见 TaikoPlayfield.cs）。
# 之前直接拿 1080p 下的整段本地宽度 1109 来画 80px 高的行，只缩了高度没缩宽，
# 会让同一帧里一行视觉上比游戏更长、同屏 note 数偏少。
GIF_TAIKO_BASE_HEIGHT = 200.0
GIF_REFERENCE_SCROLL_LENGTH = 1109.3333333333333
GIF_REFERENCE_JUDGEMENT_X = 76.0
GIF_JUDGEMENT_LINE_OFFSET = round(GIF_REFERENCE_JUDGEMENT_X * GIF_ROW_HEIGHT / GIF_TAIKO_BASE_HEIGHT)
# osu! taiko 1080p 16:9 下的时间范围参数（来自 TaikoPlayfieldAdjustmentContainer.cs）
GIF_STABLE_GAMEFIELD_HEIGHT = 480.0
GIF_STABLE_HIT_LOCATION = 160.0
GIF_VELOCITY_MULTIPLIER = 1.4
GIF_ASPECT = 16.0 / 9.0  # 1080p 16:9
GIF_SCROLL_LENGTH_PX = round(GIF_REFERENCE_SCROLL_LENGTH * GIF_ROW_HEIGHT / GIF_TAIKO_BASE_HEIGHT)
