from __future__ import annotations

#region png相关
PIXELS_PER_MS = 0.4  # 纵向时间轴像素密度
MAX_AREA_HEIGHT_0_TO_1_MIN = 4000  # [0, 1) 分钟最大区域高度
MAX_AREA_HEIGHT_1_TO_2_MIN = 5500  # [1, 2) 分钟最大区域高度
MAX_AREA_HEIGHT_2_TO_3_MIN = 7000  # [2, 3) 分钟最大区域高度
MAX_AREA_HEIGHT_3_TO_4_MIN = 8500  # [3, 4) 分钟最大区域高度
MAX_AREA_HEIGHT_4_TO_5_MIN = 10000  # [4, 5) 分钟最大区域高度
MAX_AREA_HEIGHT_5_TO_6_MIN = 11500  # [5, 6) 分钟最大区域高度
FIXED_COLUMN_COUNT_6_TO_10_MIN = 30  # [6, 10) 分钟固定列数
MAX_SUPPORTED_DURATION_MS = 10 * 60 * 1000  # 支持渲染的最大谱面时长
PAGE_MARGIN_X = 20  # 图片左右外边距
PAGE_MARGIN_Y = 20  # 图片上下外边距
LANE_WIDTH = 38  # 单个轨道宽度
LANE_GAP = 0  # PNG 轨道之间间距
COLUMN_GAP = 100  # 列与列之间间距
NOTE_HEAD_HEIGHT = 15  # 长条头部高度
BOTTOM_PADDING_MS = 2000  # 谱面底部额外预留时间
TOP_BUFFER = NOTE_HEAD_HEIGHT  # 顶部额外缓冲高度
LEFT_PANEL_WIDTH = 12  # 轨道左侧区域宽度
NOTE_SIDE_PADDING = 2  # note 左右内边距
TIME_LABEL_FONT_SIZE = 20  # 时间标签字号
SV_TEXT_FONT_SIZE = 10  # SV 文字字号
#endregion

#region gif相关
GIF_SEGMENT_COUNT = 4  # GIF 横向预览段数
GIF_DURATION_MS = 10000  # 每段播放时长(ms)
GIF_FPS = 15  # GIF 帧率
GIF_LOOP = 0  # GIF 循环次数，0 表示无限循环
GIF_SCROLL_SPEED = 33.0  # 默认 mania scroll speed
GIF_MAX_TIME_RANGE = 11485.0  # osu! DrawableManiaRuleset.MAX_TIME_RANGE
GIF_FRAME_HEIGHT = 768  # 单段 GIF 高度，对齐 osu! 1080p 下的 768 高度坐标系
GIF_GRID_GAP = 128  # GIF 段间距
GIF_SEPARATOR_WIDTH = 10  # GIF 段间分割区域宽度
GIF_HIT_TARGET_FROM_BOTTOM = 110  # osu!mania 默认判定线离底部距离
GIF_DEFAULT_HIT_POSITION = 124.8  # osu!mania legacy 默认 HitPosition，用于计算 33 速可视时间修正
GIF_STAGE_TOP_PADDING = 16  # GIF 轨道顶部预留区域高度
GIF_TIME_LABEL_HEIGHT = 38  # GIF 时间标签预留高度
GIF_TIME_LABEL_TOP_GAP = 5  # GIF 轨道与时间标签之间的间距
GIF_TIME_LABEL_FONT_SIZE = 20  # GIF 主时间标签字号
GIF_TIME_LABEL_NOTE_FONT_SIZE = 14  # GIF PreviewTime 备注字号
GIF_TIME_LABEL_COLOR = (232, 232, 232, 255)  # GIF 普通时间标签颜色
GIF_TIME_LABEL_NOTE_COLOR = (170, 170, 170, 255)  # GIF 普通备注标签颜色
GIF_PREVIEW_TIME_LABEL_COLOR = (95, 221, 108, 255)  # GIF PreviewTime 标签颜色
GIF_JUDGEMENT_LINE = (238, 238, 238, 255)  # GIF 判定线颜色
GIF_SEPARATOR_BACKGROUND = (8, 8, 8, 255)  # GIF 段间分割区域背景色
#endregion

LEFT_PANEL_BACKGROUND = (112, 112, 112, 255)  # 轨道左侧区域背景色
IMAGE_BACKGROUND = (0, 0, 0, 255)  # 整体背景色
LANE_BACKGROUND = (0, 0, 0, 255)  # 轨道背景色
RULER_TEXT = (232, 232, 232, 255)  # 时间文字颜色
MEASURE_LINE = (83, 83, 83)  # (220,220,220,96) 预混合到黑色背景
BEAT_LINE = (56, 56, 56)  # (200,200,200,72) 预混合到黑色背景
SUBDIVISION_LINE = (34, 34, 34)  # (180,180,180,48) 预混合到黑色背景
LANE_SEPARATOR = (32, 32, 32, 255)  # PNG 轨道分隔线颜色
SV_TEXT_COLOR = (95, 221, 108, 255)  # SV 文字显示颜色
