from __future__ import annotations

#region png相关
PNG_MS_PER_IMAGE = 400  # 一行内两张 PNG 图片的时间间隔(ms)
PNG_ROW_COUNT = 5  # PNG 行数
PNG_IMAGES_PER_ROW = 8  # PNG 每行图片数量
#endregion

#region gif相关
GIF_ROW_COUNT = 2  # GIF 行数
GIF_IMAGES_PER_ROW = 2  # GIF 每行图片数量
GIF_DURATION_MS = 5000  # GIF 每个时间段的播放时长(ms)
GIF_FPS = 15  # GIF 帧率
GIF_LOOP = 0  # GIF 循环次数，0 表示无限循环
GIF_GRID_GAP = 20  # GIF 宫格间距
#endregion

IMAGE_WIDTH = 512  # 每张预览图片宽度
IMAGE_HEIGHT = 384  # 每张预览图片高度
HORIZONTAL_PAGE_MARGIN = 20  # 图片左右外边距
VERTICAL_PAGE_MARGIN = 20  # 图片上下外边距
INTRA_ROW_IMAGE_GAP = 20  # 一行内两张图片之间间距
INTER_ROW_GAP = 100  # 行与行之间间距
LEFT_PANEL_WIDTH = 12  # 每张图片左侧区域宽度
LEFT_PANEL_BACKGROUND_COLOR = (112, 112, 112, 255)  # 每张图片左侧区域背景色
CANVAS_BACKGROUND_COLOR = (0, 0, 0, 255)  # 整体背景色
IMAGE_BACKGROUND_COLOR = (0, 0, 0, 255)  # 每张图片背景色

TIME_LABEL_FONT_SIZE = 30  # 时间标签字号，约为 PIL 默认字号的三倍
TIME_LABEL_NOTE_FONT_SIZE = 22  # 时间标签备注字号，用于 PreviewTime 说明
TIME_LABEL_HEIGHT = 76  # 时间标签预留高度，包含主时间和可选备注
TIME_LABEL_TOP_GAP = 8  # 图片与时间标签之间的间距
TIME_LABEL_NOTE_TOP_GAP = 9  # 主时间与备注之间的间距
TIME_LABEL_COLOR = (232, 232, 232, 255)  # 时间标签文字颜色
TIME_LABEL_NOTE_COLOR = (170, 170, 170, 255)  # 时间标签备注文字颜色
PREVIEW_TIME_LABEL_COLOR = (95, 221, 108, 255)  # PreviewTime 所在时间标签和备注颜色
