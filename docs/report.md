# 批量渲染报告

本文件汇总三类批量渲染任务的原始报告。每份报告均保留生成脚本输出的完整内容，便于对比不同渲染流程的耗时、资源占用和成功率。

## Rust 批处理报告

该部分对应 `batch-rust/report.txt`，记录 Rust 批量渲染 PNG 和 GIF（包括 Mod、转谱及时间点组合）的任务耗时、峰值内存、输出大小和 CPU 使用率。

```text
osu-beatmap-preview configuration render report
Generated: 2026-09-18 20:14:12
Output: C:\Users\27101\AppData\Local\Temp\osu-beatmap-preview\outputs\batch-config
Tasks: 47  Success: 47  Failed: 0
Total measured time: 34209ms (34.209s)  Peak memory: 335.6MB

  # MODE     LABEL                                    STATUS   RESOLUTION       TIME   PEAKMEM       SIZE     CPU
-----------------------------------------------------------------------------------------------------------------------------
  1 standard standard_gif_no_time.gif                 success  1120x828        849ms    97.1MB   2059.9KB  187.7%
  2 standard standard_gif_no_time_1x1.gif             success  570x424         281ms    40.9MB    533.5KB  166.8%
  3 standard standard_png_0.5x.png                    success  2210x1390       157ms    27.6MB    172.2KB   59.7%
  4 standard standard_png_1x.png                      success  4420x2780       292ms    74.4MB    425.8KB     91%
  5 standard standard_png_2x.png                      success  8840x5560       885ms   259.7MB     1022KB   95.3%
  6 standard standard_gif_0.5x.gif                    success  560x498         228ms    38.6MB    210.6KB  205.6%
  7 standard standard_gif_1x.gif                      success  1120x996        945ms   106.2MB   2150.3KB  213.3%
  8 standard standard_gif_2x.gif                      success  2240x1992      2755ms   234.4MB   3438.7KB  170.7%
  9 standard standard_mp4_0.5x.mp4                    success  342x192         913ms   148.6MB     3747KB  195.1%
 10 standard standard_mp4_1x.mp4                      success  684x384        1005ms   206.6MB   3640.3KB  312.5%
 11 standard standard_mp4_2x.mp4                      success  1366x768       1798ms   335.6MB   3755.8KB  455.4%
 12 taiko    taiko_gif_no_time.gif                    success  699x437         293ms    24.9MB   1182.1KB     80%
 13 taiko    taiko_gif_no_time_1x1.gif                success  699x116         103ms    14.8MB    308.3KB   75.8%
 14 taiko    taiko_png_0.5x.png                       success  2236x1643        91ms    28.8MB    102.7KB   68.7%
 15 taiko    taiko_png_1x.png                         success  4473x3286       190ms    84.2MB    223.4KB   90.5%
 16 taiko    taiko_png_2x.png                         success  8945x6572       561ms   306.1MB    502.3KB  100.3%
 17 taiko    taiko_gif_0.5x.gif                       success  349x284         136ms    15.8MB    466.8KB   91.9%
 18 taiko    taiko_gif_1x.gif                         success  699x569         328ms      29MB   1234.2KB  128.6%
 19 taiko    taiko_gif_2x.gif                         success  1397x1138      1069ms    84.1MB   3133.7KB  146.2%
 20 taiko    taiko_mp4_0.5x.mp4                       success  342x192         723ms   126.1MB   2466.8KB  162.1%
 21 taiko    taiko_mp4_1x.mp4                         success  684x386         913ms   146.4MB   3316.1KB  241.3%
 22 taiko    taiko_mp4_2x.mp4                         success  1366x768       1813ms   239.8MB   4309.6KB    281%
 23 catch    catch_gif_no_time.gif                    success  990x818         624ms    46.7MB   1356.2KB  127.7%
 24 catch    catch_gif_no_time_1x1.gif                success  500x414         170ms      24MB      330KB  174.6%
 25 catch    catch_png_0.5x.png                       success  1576x1888        91ms    21.7MB    151.6KB   85.9%
 26 catch    catch_png_1x.png                         success  3135x3774       187ms    70.1MB    391.7KB   91.9%
 27 catch    catch_png_2x.png                         success  6270x7539       505ms   248.7MB    869.8KB   92.8%
 28 catch    catch_gif_0.5x.gif                       success  496x494         229ms    21.5MB    603.3KB  109.2%
 29 catch    catch_gif_1x.gif                         success  990x986         667ms      53MB   1397.5KB  182.7%
 30 catch    catch_gif_2x.gif                         success  1980x1972      2507ms   100.6MB   3179.1KB  132.8%
 31 catch    catch_mp4_0.5x.mp4                       success  342x192         667ms    99.5MB   3681.1KB  175.7%
 32 catch    catch_mp4_1x.mp4                         success  684x384         854ms     124MB   3677.1KB  232.4%
 33 catch    catch_mp4_2x.mp4                         success  1366x768       1805ms     213MB   4474.2KB  292.6%
 34 mania    mania_gif_no_time.gif                    success  972x424         504ms    28.2MB      347KB   99.2%
 35 mania    mania_gif_no_time_1x1.gif                success  228x424         157ms      15MB    133.7KB   79.6%
 36 mania    mania_png_0.5x.png                       success  1131x3144        70ms    24.1MB     38.5KB   89.3%
 37 mania    mania_png_1x.png                         success  2262x6289       160ms    81.5MB     87.5KB   97.7%
 38 mania    mania_png_2x.png                         success  4524x12578      476ms   295.5MB    166.3KB   95.2%
 39 mania    mania_gif_0.5x.gif                       success  486x236         204ms    15.6MB    147.1KB  199.1%
 40 mania    mania_gif_1x.gif                         success  972x472         507ms    30.4MB    348.2KB    114%
 41 mania    mania_gif_2x.gif                         success  1944x944       1940ms    88.2MB    819.3KB  103.9%
 42 mania    mania_mp4_0.5x.mp4                       success  342x192         607ms     106MB   2088.8KB  180.2%
 43 mania    mania_mp4_1x.mp4                         success  684x384         843ms   130.6MB   2407.2KB  194.6%
 44 mania    mania_mp4_2x.mp4                         success  1366x768       1806ms   220.8MB   3592.8KB  285.5%
 45 mania    mania_png_no_sv.png                      success  2262x6289       197ms    81.4MB     50.7KB  103.1%
 46 mania    mania_gif_no_sv_30fps.gif                success  972x472         786ms    29.8MB    412.6KB  147.1%
 47 mania    mania_mp4_no_sv_30fps.mp4                success  684x384        1318ms   130.3MB   2511.7KB  221.7%
```

## 视频批处理报告

该部分对应 `batch-video/report.txt`，记录四种游戏模式生成 MP4 视频时的渲染耗时、GPU/CPU 使用率、进程内存和输出文件大小。

```text
osu-beatmap-preview full MP4 benchmark
Generated: 2026-09-18 20:13:14
Binary: E:\MyCodes\rust\osu-beatmap-preview\target\release\osu-beatmap-preview-cli.exe
Output: C:\Users\27101\AppData\Local\Temp\osu-beatmap-preview\outputs\batch-video
NoCache: False
GPU sampling interval: 500ms
GPU scope: process = Windows per-process GPU Engine; system = nvidia-smi whole GPU

Tasks: 13  Success: 13  Failed: 0
Total chart/video duration: 2098.066s
Total wall time: 47413.5ms (47.41s)
Overall average render cost: 22.60ms per chart-second
Peak GPU: 43.0%  Peak process memory: 546.2MB

GPU AVG includes download, audio preparation, rendering, and final mux wait.
GPU ACTIVE AVG excludes samples at or below 0.5%.

  # MODE   BID       STATUS     CHART(s)    WALL(ms)  ms/chart-s   GPU AVG   GPU ACTIVE   GPU PEAK      CPU    MEM MB   SIZE MB
------------------------------------------------------------------------------------------------------------------------------------------------------
  1 std    5242890   success      35.133      1818.2       51.75      0.0%         0.0%       0.0%   354.1%       4.0      4.40
  2 std    4897202   success     149.467      3323.0       22.23     14.5%        29.0%      29.0%   397.8%     285.8     18.94
  3 std    1024742   success     320.333      7337.9       22.91     27.3%        29.6%      42.0%   379.7%     546.2     41.25
  4 taiko  5619629   success      94.600      2010.8       21.26     21.2%        28.3%      30.0%   253.3%     156.8      8.62
  5 taiko  5175577   success     128.267      2760.2       21.52     20.2%        25.2%      27.0%   255.3%     173.3     15.15
  6 taiko  1418246   success     286.333      6172.1       21.56     26.5%        29.2%      35.0%   264.8%     259.4     35.67
  7 ctb    944502    success      41.467      1770.2       42.69     15.5%        20.7%      29.0%   227.7%     115.5      5.09
  8 ctb    2103068   success      92.600      1954.0       21.10     19.8%        26.3%      29.0%   280.7%     146.2     11.61
  9 ctb    2182842   success     241.933      4909.2       20.29     25.1%        28.2%      30.0%   307.8%     229.3     30.87
 10 mania  4624418   success      40.933      1044.0       25.51      6.0%        12.0%      12.0%   215.5%     111.0      3.34
 11 mania  5572554   success     142.267      2791.0       19.62     22.0%        27.5%      31.0%   262.0%     174.6     10.96
 12 mania  3562727   success      97.600      2073.8       21.25     20.8%        27.7%      30.0%   239.6%     150.5      7.49
 13 mania  4312004   success     427.133      9449.1       22.12     26.5%        30.1%      43.0%   304.4%     338.0     40.84

Per-mode summary:
  ctb    count= 3 duration=  376.000s wall=    8633.4ms cost=   22.96ms/chart-s avgGPU= 20.1%
  mania  count= 4 duration=  707.933s wall=   15357.9ms cost=   21.69ms/chart-s avgGPU= 18.8%
  std    count= 3 duration=  504.933s wall=   12479.1ms cost=   24.71ms/chart-s avgGPU= 13.9%
  taiko  count= 3 duration=  509.200s wall=   10943.1ms cost=   21.49ms/chart-s avgGPU= 22.6%
```

## 配置批处理报告

该部分对应 `batch-config/report.txt`，覆盖不同游戏模式、输出格式、分辨率、帧率、Mod、转谱和时间点配置，用于验证配置组合的渲染结果与资源占用。

```text
osu-beatmap-preview configuration render report
Generated: 2026-09-18 20:14:12
Output: C:\Users\27101\AppData\Local\Temp\osu-beatmap-preview\outputs\batch-config
Tasks: 47  Success: 47  Failed: 0
Total measured time: 34209ms (34.209s)  Peak memory: 335.6MB

  # MODE     LABEL                                    STATUS   RESOLUTION       TIME   PEAKMEM       SIZE     CPU
-----------------------------------------------------------------------------------------------------------------------------
  1 standard standard_gif_no_time.gif                 success  1120x828        849ms    97.1MB   2059.9KB  187.7%
  2 standard standard_gif_no_time_1x1.gif             success  570x424         281ms    40.9MB    533.5KB  166.8%
  3 standard standard_png_0.5x.png                    success  2210x1390       157ms    27.6MB    172.2KB   59.7%
  4 standard standard_png_1x.png                      success  4420x2780       292ms    74.4MB    425.8KB     91%
  5 standard standard_png_2x.png                      success  8840x5560       885ms   259.7MB     1022KB   95.3%
  6 standard standard_gif_0.5x.gif                    success  560x498         228ms    38.6MB    210.6KB  205.6%
  7 standard standard_gif_1x.gif                      success  1120x996        945ms   106.2MB   2150.3KB  213.3%
  8 standard standard_gif_2x.gif                      success  2240x1992      2755ms   234.4MB   3438.7KB  170.7%
  9 standard standard_mp4_0.5x.mp4                    success  342x192         913ms   148.6MB     3747KB  195.1%
 10 standard standard_mp4_1x.mp4                      success  684x384        1005ms   206.6MB   3640.3KB  312.5%
 11 standard standard_mp4_2x.mp4                      success  1366x768       1798ms   335.6MB   3755.8KB  455.4%
 12 taiko    taiko_gif_no_time.gif                    success  699x437         293ms    24.9MB   1182.1KB     80%
 13 taiko    taiko_gif_no_time_1x1.gif                success  699x116         103ms    14.8MB    308.3KB   75.8%
 14 taiko    taiko_png_0.5x.png                       success  2236x1643        91ms    28.8MB    102.7KB   68.7%
 15 taiko    taiko_png_1x.png                         success  4473x3286       190ms    84.2MB    223.4KB   90.5%
 16 taiko    taiko_png_2x.png                         success  8945x6572       561ms   306.1MB    502.3KB  100.3%
 17 taiko    taiko_gif_0.5x.gif                       success  349x284         136ms    15.8MB    466.8KB   91.9%
 18 taiko    taiko_gif_1x.gif                         success  699x569         328ms      29MB   1234.2KB  128.6%
 19 taiko    taiko_gif_2x.gif                         success  1397x1138      1069ms    84.1MB   3133.7KB  146.2%
 20 taiko    taiko_mp4_0.5x.mp4                       success  342x192         723ms   126.1MB   2466.8KB  162.1%
 21 taiko    taiko_mp4_1x.mp4                         success  684x386         913ms   146.4MB   3316.1KB  241.3%
 22 taiko    taiko_mp4_2x.mp4                         success  1366x768       1813ms   239.8MB   4309.6KB    281%
 23 catch    catch_gif_no_time.gif                    success  990x818         624ms    46.7MB   1356.2KB  127.7%
 24 catch    catch_gif_no_time_1x1.gif                success  500x414         170ms      24MB      330KB  174.6%
 25 catch    catch_png_0.5x.png                       success  1576x1888        91ms    21.7MB    151.6KB   85.9%
 26 catch    catch_png_1x.png                         success  3135x3774       187ms    70.1MB    391.7KB   91.9%
 27 catch    catch_png_2x.png                         success  6270x7539       505ms   248.7MB    869.8KB   92.8%
 28 catch    catch_gif_0.5x.gif                       success  496x494         229ms    21.5MB    603.3KB  109.2%
 29 catch    catch_gif_1x.gif                         success  990x986         667ms      53MB   1397.5KB  182.7%
 30 catch    catch_gif_2x.gif                         success  1980x1972      2507ms   100.6MB   3179.1KB  132.8%
 31 catch    catch_mp4_0.5x.mp4                       success  342x192         667ms    99.5MB   3681.1KB  175.7%
 32 catch    catch_mp4_1x.mp4                         success  684x384         854ms     124MB   3677.1KB  232.4%
 33 catch    catch_mp4_2x.mp4                         success  1366x768       1805ms     213MB   4474.2KB  292.6%
 34 mania    mania_gif_no_time.gif                    success  972x424         504ms    28.2MB      347KB   99.2%
 35 mania    mania_gif_no_time_1x1.gif                success  228x424         157ms      15MB    133.7KB   79.6%
 36 mania    mania_png_0.5x.png                       success  1131x3144        70ms    24.1MB     38.5KB   89.3%
 37 mania    mania_png_1x.png                         success  2262x6289       160ms    81.5MB     87.5KB   97.7%
 38 mania    mania_png_2x.png                         success  4524x12578      476ms   295.5MB    166.3KB   95.2%
 39 mania    mania_gif_0.5x.gif                       success  486x236         204ms    15.6MB    147.1KB  199.1%
 40 mania    mania_gif_1x.gif                         success  972x472         507ms    30.4MB    348.2KB    114%
 41 mania    mania_gif_2x.gif                         success  1944x944       1940ms    88.2MB    819.3KB  103.9%
 42 mania    mania_mp4_0.5x.mp4                       success  342x192         607ms     106MB   2088.8KB  180.2%
 43 mania    mania_mp4_1x.mp4                         success  684x384         843ms   130.6MB   2407.2KB  194.6%
 44 mania    mania_mp4_2x.mp4                         success  1366x768       1806ms   220.8MB   3592.8KB  285.5%
 45 mania    mania_png_no_sv.png                      success  2262x6289       197ms    81.4MB     50.7KB  103.1%
 46 mania    mania_gif_no_sv_30fps.gif                success  972x472         786ms    29.8MB    412.6KB  147.1%
 47 mania    mania_mp4_no_sv_30fps.mp4                success  684x384        1318ms   130.3MB   2511.7KB  221.7%
```
