# 批量渲染报告

本文件汇总三类批量渲染任务的原始报告。每份报告均保留生成脚本输出的完整内容，便于对比不同渲染流程的耗时、资源占用和成功率。

## Rust 批处理报告

该部分对应 `batch-rust/report.txt`，记录 Rust 批量渲染 PNG 和 GIF（包括 Mod、转谱及时间点组合）的任务耗时、峰值内存、输出大小和 CPU 使用率。

```text
osu-beatmap-preview (Rust) 批量渲染报告
生成时间: 2026-09-01 23:43:51
任务总数: 90    成功: 90    失败: 0
总耗时: 62092ms (62.1s)    峰值内存(单进程最大): 890.3MB
CPU 统计: 平均 134.8%    最高 502.8%    总CPU时间 124.6s

  #  MODE    LABEL                                      STATUS        TIME   PEAKMEM        SIZE       %
--------------------------------------------------------------------------------------------------------------
  1  std     standard_738063.png                        success      416ms    74.9MB     402.8KB   93.9%
  2  std     standard_2875069.png                       success      332ms      75MB       324KB   94.1%
  3  std     standard_4897202.png                       success      383ms    75.3MB     346.8KB   89.8%
  4  std     standard_1024742.png                       success      364ms      76MB     427.7KB   85.9%
  5  std     standard_372245.png                        success      772ms    78.4MB    1527.1KB   89.1%
  6  std     standard_1529760.png                       success     2444ms   143.9MB     761.6KB   97.8%
  7  std     standard_5467386.png                       success      662ms    81.7MB     212.7KB   99.1%
  8  std     standard_738063.gif                        success     1166ms   119.8MB    1972.7KB  280.1%
  9  std     standard_2875069.gif                       success     1104ms    99.8MB    1562.3KB  250.5%
 10  std     standard_4897202.gif                       success     1020ms    93.7MB    1869.8KB  220.6%
 11  std     standard_1024742.gif                       success     1055ms   111.2MB    2138.6KB    237%
 12  std     standard_372245.gif                        success     1757ms   123.9MB    7405.6KB  336.2%
 13  std     standard_1529760.gif                       success     6688ms   890.3MB    3904.9KB  502.8%
 14  std     standard_5467386.gif                       success     1545ms   125.2MB    1173.5KB  338.8%
 15  taiko   taiko_4242023.png                          success      212ms      86MB     199.6KB   88.4%
 16  taiko   taiko_1418246.png                          success      400ms   122.6MB     488.9KB   85.9%
 17  taiko   taiko_4590053.png                          success      178ms    63.2MB     115.5KB     79%
 18  taiko   taiko_2923535.png                          success      303ms      88MB     354.9KB   82.5%
 19  taiko   taiko_5651058.png                          success      604ms   246.2MB     507.3KB     88%
 20  taiko   taiko_3726150.png                          success      350ms   119.8MB     202.2KB   89.3%
 21  taiko   taiko_4242023.gif                          success      405ms    28.9MB    1079.2KB  100.3%
 22  taiko   taiko_1418246.gif                          success      481ms    31.8MB    1365.3KB  107.2%
 23  taiko   taiko_4590053.gif                          success      369ms    29.2MB     824.4KB   93.2%
 24  taiko   taiko_2923535.gif                          success      469ms    33.7MB    2409.1KB  126.6%
 25  taiko   taiko_5651058.gif                          success      364ms    30.3MB    1169.2KB   85.9%
 26  taiko   taiko_3726150.gif                          success      321ms    29.1MB     790.1KB   92.5%
 27  catch   catch_3852338.png                          success      186ms    58.6MB     257.1KB     84%
 28  catch   catch_3807626.png                          success      369ms   156.3MB     768.4KB   80.5%
 29  catch   catch_944502.png                           success      203ms    43.9MB     531.6KB  100.1%
 30  catch   catch_2571609.png                          success      713ms   345.1MB    1199.7KB   96.4%
 31  catch   catch_265177.png                           success      271ms    96.9MB     683.9KB   86.5%
 32  catch   catch_3852338.gif                          success      691ms    57.8MB    1212.6KB  144.7%
 33  catch   catch_3807626.gif                          success      776ms    60.1MB    1405.2KB    155%
 34  catch   catch_944502.gif                           success      986ms    67.1MB    5244.3KB  160.1%
 35  catch   catch_2571609.gif                          success      800ms    60.6MB     700.5KB  132.8%
 36  catch   catch_265177.gif                           success      942ms    63.5MB    2394.2KB  159.2%
 37  mania   mania_4312004.png                          success      479ms   227.5MB       261KB   84.8%
 38  mania   mania_4610729.png                          success      180ms    75.6MB      60.1KB   86.8%
 39  mania   mania_5061439.png                          success      182ms    62.3MB      68.5KB   94.4%
 40  mania   mania_4789195.png                          success      367ms   146.2MB     381.7KB   89.4%
 41  mania   mania_3793380.png                          success      278ms   119.8MB     228.9KB   95.5%
 42  mania   mania_4665942.png                          success      234ms   100.7MB      79.8KB   86.8%
 43  mania   mania_5354177.png                          success      307ms   128.7MB       102KB   81.4%
 44  mania   mania_5221843.png                          success      150ms    47.8MB       103KB   72.9%
 45  mania   mania_5369780.png                          success      245ms   127.2MB      91.8KB   89.3%
 46  mania   mania_4972672.png                          success      369ms   192.4MB       145KB   97.4%
 47  mania   mania_5013742.png                          success      358ms   183.3MB      82.7KB   87.3%
 48  mania   mania_4312004.gif                          success      650ms    31.8MB    1111.5KB  146.6%
 49  mania   mania_4610729.gif                          success      604ms    30.8MB     381.9KB  108.7%
 50  mania   mania_5061439.gif                          success      677ms    30.7MB     595.3KB  115.4%
 51  mania   mania_4789195.gif                          success      690ms    35.7MB     823.2KB   90.6%
 52  mania   mania_3793380.gif                          success      585ms    33.6MB     541.3KB   93.5%
 53  mania   mania_4665942.gif                          success      721ms    33.2MB     756.3KB     91%
 54  mania   mania_5354177.gif                          success      614ms    31.9MB     310.1KB  106.9%
 55  mania   mania_5221843.gif                          success      798ms    35.9MB     934.4KB  115.5%
 56  mania   mania_5369780.gif                          success      741ms    34.4MB     393.2KB  105.4%
 57  mania   mania_4972672.gif                          success      938ms    40.1MB     614.2KB  108.3%
 58  mania   mania_5013742.gif                          success     1192ms    52.6MB     351.6KB  128.5%
 59  std     standard_738063_hd-hr.gif                  success      987ms   108.9MB     897.6KB    258%
 60  std     standard_2875069_hr.png                    success      340ms    74.8MB     198.8KB   91.9%
 61  std     standard_4897202_dt1.3.gif                 success     1116ms   100.7MB    1961.8KB  246.4%
 62  std     standard_1024742_daar9.5-dacs4.5.gif       success     1109ms     111MB    2077.8KB  256.4%
 63  std     standard_5467386_ez-hd.gif                 success     2115ms   189.5MB    1837.8KB  388.6%
 64  taiko   taiko_4242023_hr.gif                       success      322ms    29.1MB     673.1KB  111.6%
 65  taiko   taiko_1418246_dt.gif                       success      368ms    31.9MB    1092.6KB  148.6%
 66  taiko   taiko_4590053_sw.png                       success      160ms      63MB       117KB   78.1%
 67  taiko   taiko_2923535_cs.gif                       success      426ms    33.8MB    2272.4KB    154%
 68  catch   catch_3852338_hr.gif                       success      660ms      56MB     725.9KB  144.4%
 69  catch   catch_3807626_ez.png                       success      303ms    87.5MB     959.4KB  103.1%
 70  catch   catch_944502_dt1.4.gif                     success     1084ms      66MB    5316.9KB    160%
 71  mania   mania_4312004_in.png                       success      563ms   227.3MB     260.3KB     86%
 72  mania   mania_4610729_ho.gif                       success      634ms    30.6MB     381.9KB   91.2%
 73  mania   mania_5061439_cs.gif                       success      618ms    30.6MB     552.4KB  103.7%
 74  convert mania_5473947_convert_ds.gif               success     1138ms    48.3MB     625.4KB  111.2%
 75  convert taiko_738063_convert.png                   success      150ms    57.5MB      74.6KB   72.9%
 76  convert taiko_2875069_convert.gif                  success      314ms    29.4MB       670KB  134.4%
 77  convert catch_4897202_convert.png                  success      303ms   120.8MB     565.8KB   82.5%
 78  convert catch_1024742_convert.gif                  success      776ms    66.8MB    1271.5KB  173.2%
 79  convert mania_372245_convert.png                   success      202ms    73.1MB     103.8KB   77.4%
 80  convert mania_1529760_convert.gif                  success      311ms    34.4MB        67KB  160.8%
 81  convert taiko_5467386_convert.gif                  success      331ms    33.8MB     186.5KB  122.7%
 82  convert taiko_260177_convert.png                   success      177ms    64.5MB      88.9KB   88.3%
 83  convert catch_260177_convert.png                   success      274ms   146.4MB     254.4KB   91.2%
 84  std     standard_738063_time-points30-40-50-60.gif success     1143ms   122.6MB    1962.4KB  292.5%
 85  std     standard_2875069_time-points10-25-60.gif   success     1049ms   101.9MB    1596.7KB  244.3%
 86  std     standard_4897202_time-points45.gif         success      989ms    94.1MB    1930.1KB  210.1%
 87  convert mania_738063_convert_in.gif                success      715ms    34.1MB     707.9KB  124.6%
 88  convert catch_2875069_convert_hr.gif               success      682ms    58.2MB     561.1KB  114.6%
 89  std     standard_4897202_hd-dt1.25_time-points20-40.gif success      968ms   100.9MB     954.6KB  216.3%
 90  convert taiko_5467386_convert_hr_time-points15-30.gif success      305ms    33.5MB     120.3KB   87.1%
```

## 视频批处理报告

该部分对应 `batch-video/report.txt`，记录四种游戏模式生成 MP4 视频时的渲染耗时、GPU/CPU 使用率、进程内存和输出文件大小。

```text
osu-beatmap-preview full MP4 benchmark
Generated: 2026-09-01 23:45:36
Binary: E:\MyCodes\rust\osu-beatmap-preview\target\release\osu-beatmap-preview.exe
Output: C:\Users\27101\AppData\Local\Temp\osu-beatmap-preview\outputs\batch-video
NoCache: False
GPU sampling interval: 500ms
GPU scope: process = Windows per-process GPU Engine; system = nvidia-smi whole GPU

Tasks: 13  Success: 13  Failed: 0
Total chart/video duration: 2098.066s
Total wall time: 70518.9ms (70.52s)
Overall average render cost: 33.61ms per chart-second
Peak GPU: 47.2%  Peak process memory: 507.4MB

GPU AVG includes download, audio preparation, rendering, and final mux wait.
GPU ACTIVE AVG excludes samples at or below 0.5%.

  # MODE   BID       STATUS     CHART(s)    WALL(ms)  ms/chart-s   GPU AVG   GPU ACTIVE   GPU PEAK      CPU    MEM MB   SIZE MB
------------------------------------------------------------------------------------------------------------------------------------------------------
  1 std    5242890   success      35.133      2377.5       67.67      0.0%         0.0%       0.0%   372.0%       4.4      4.13
  2 std    4897202   success     149.467      6991.5       46.78     39.1%        39.1%      47.2%   227.5%     206.7     17.76
  3 std    1024742   success     320.333     19035.8       59.43     20.1%        24.2%      35.8%   199.0%     507.4     38.80
  4 taiko  5619629   success      94.600      2312.1       24.44     16.8%        22.3%      26.0%   150.7%     106.6      7.94
  5 taiko  5175577   success     128.267      2823.1       22.01     20.8%        20.8%      26.0%   140.6%     110.8     14.21
  6 taiko  1418246   success     286.333      7942.5       27.74     28.3%        30.5%      37.0%   123.9%     135.2     33.49
  7 ctb    944502    success      41.467      1374.3       33.14      6.0%         9.0%      17.0%   205.8%      98.8      4.84
  8 ctb    2103068   success      92.600      2591.4       27.98     16.0%        20.0%      22.0%   202.6%     108.5     10.68
  9 ctb    2182842   success     241.933      7297.0       30.16     26.6%        28.8%      37.0%   152.0%     145.8     28.94
 10 mania  4624418   success      40.933      1183.5       28.91      5.7%        17.0%      17.0%   188.8%      76.6      3.05
 11 mania  5572554   success     142.267      2984.3       20.98     20.0%        24.0%      26.0%   161.8%     108.3     10.09
 12 mania  3562727   success      97.600      2186.7       22.41     13.0%        17.3%      26.0%   164.3%     101.3      6.83
 13 mania  4312004   success     427.133     11419.2       26.73     30.6%        32.3%      41.0%   117.7%     220.6     38.91

Per-mode summary:
  ctb    count= 3 duration=  376.000s wall=   11262.7ms cost=   29.95ms/chart-s avgGPU= 16.2%
  mania  count= 4 duration=  707.933s wall=   17773.7ms cost=   25.11ms/chart-s avgGPU= 17.3%
  std    count= 3 duration=  504.933s wall=   28404.8ms cost=   56.25ms/chart-s avgGPU= 19.7%
  taiko  count= 3 duration=  509.200s wall=   13077.7ms cost=   25.68ms/chart-s avgGPU= 22.0%
```

## 配置批处理报告

该部分对应 `batch-config/report.txt`，覆盖不同游戏模式、输出格式、分辨率、帧率、Mod、转谱和时间点配置，用于验证配置组合的渲染结果与资源占用。

```text
osu-beatmap-preview configuration render report
Generated: 2026-09-01 23:42:23
Output: C:\Users\27101\AppData\Local\Temp\osu-beatmap-preview\outputs\batch-config
Tasks: 47  Success: 47  Failed: 0
Total measured time: 41700ms (41.700s)  Peak memory: 316.4MB

  # MODE     LABEL                                    STATUS   RESOLUTION       TIME   PEAKMEM       SIZE     CPU
-----------------------------------------------------------------------------------------------------------------------------
  1 standard standard_gif_no_time.gif                 success  1120x828       1471ms   104.7MB   2134.2KB  180.6%
  2 standard standard_gif_no_time_1x1.gif             success  570x424         297ms    43.8MB      532KB  236.7%
  3 standard standard_png_0.5x.png                    success  2210x1390       172ms    28.4MB    168.3KB   90.8%
  4 standard standard_png_1x.png                      success  4420x2780       325ms    75.6MB    427.7KB   91.3%
  5 standard standard_png_2x.png                      success  8840x5560       914ms   260.8MB   1015.9KB   97.4%
  6 standard standard_gif_0.5x.gif                    success  560x498         389ms    40.5MB    868.3KB  200.8%
  7 standard standard_gif_1x.gif                      success  1120x996       1065ms   111.3MB   2138.6KB  249.4%
  8 standard standard_gif_2x.gif                      success  2240x1992      4185ms   235.4MB   5121.8KB  160.9%
  9 standard standard_mp4_0.5x.mp4                    success  342x192        1049ms   128.3MB   3483.1KB  183.2%
 10 standard standard_mp4_1x.mp4                      success  684x384        1347ms     154MB   3405.7KB  280.7%
 11 standard standard_mp4_2x.mp4                      success  1366x768       3266ms   316.4MB   3558.9KB  335.8%
 12 taiko    taiko_gif_no_time.gif                    success  699x437         302ms    26.1MB     1189KB  124.2%
 13 taiko    taiko_gif_no_time_1x1.gif                success  699x116         133ms    16.5MB    308.3KB     94%
 14 taiko    taiko_png_0.5x.png                       success  668x11275       198ms    49.6MB    220.7KB     71%
 15 taiko    taiko_png_1x.png                         success  4473x3286       225ms    87.5MB    224.3KB   76.4%
 16 taiko    taiko_png_2x.png                         success  17726x1756      364ms   182.4MB    308.7KB   85.9%
 17 taiko    taiko_gif_0.5x.gif                       success  349x284         170ms      17MB    468.1KB  119.5%
 18 taiko    taiko_gif_1x.gif                         success  699x569         349ms    30.3MB     1230KB  111.9%
 19 taiko    taiko_gif_2x.gif                         success  1397x1138      1072ms    82.8MB   2995.3KB  112.2%
 20 taiko    taiko_mp4_0.5x.mp4                       success  342x192         665ms   125.6MB   2248.4KB  166.8%
 21 taiko    taiko_mp4_1x.mp4                         success  684x386         986ms   120.6MB   3081.1KB  163.2%
 22 taiko    taiko_mp4_2x.mp4                         success  1366x768       2076ms   162.3MB     4093KB  209.2%
 23 catch    catch_gif_no_time.gif                    success  990x818         736ms    58.8MB   1794.3KB  157.1%
 24 catch    catch_gif_no_time_1x1.gif                success  500x414         208ms    31.5MB    403.4KB  195.3%
 25 catch    catch_png_0.5x.png                       success  792x3346         98ms    25.1MB      122KB  111.6%
 26 catch    catch_png_1x.png                         success  2745x3836       195ms      66MB    403.6KB   96.2%
 27 catch    catch_png_2x.png                         success  10170x4158      569ms   227.9MB   1344.3KB   93.4%
 28 catch    catch_gif_0.5x.gif                       success  496x494         258ms    26.9MB    499.8KB  157.5%
 29 catch    catch_gif_1x.gif                         success  990x986         788ms    67.3MB   1809.3KB  184.4%
 30 catch    catch_gif_2x.gif                         success  1980x1972      2885ms   125.3MB   5581.7KB  147.9%
 31 catch    catch_mp4_0.5x.mp4                       success  342x192         827ms    75.5MB   3151.9KB  145.5%
 32 catch    catch_mp4_1x.mp4                         success  684x384        1198ms   100.8MB   3277.3KB  260.9%
 33 catch    catch_mp4_2x.mp4                         success  1366x768       2474ms   193.4MB   4186.9KB  279.2%
 34 mania    mania_gif_no_time.gif                    success  972x424         536ms    29.4MB    352.7KB  110.8%
 35 mania    mania_gif_no_time_1x1.gif                success  228x424         165ms    16.5MB    135.3KB   94.7%
 36 mania    mania_png_0.5x.png                       success  635x5641        113ms    29.6MB     40.2KB   69.1%
 37 mania    mania_png_1x.png                         success  2262x6289       189ms    83.1MB     88.2KB   74.4%
 38 mania    mania_png_2x.png                         success  8492x6703       507ms   297.1MB      166KB   95.5%
 39 mania    mania_gif_0.5x.gif                       success  486x236         199ms    17.1MB    152.7KB   78.5%
 40 mania    mania_gif_1x.gif                         success  972x472         550ms    31.5MB    353.9KB    125%
 41 mania    mania_gif_2x.gif                         success  1944x944       2041ms    88.6MB    828.1KB  114.1%
 42 mania    mania_mp4_0.5x.mp4                       success  342x192         656ms    93.1MB   1912.4KB  138.1%
 43 mania    mania_mp4_1x.mp4                         success  684x384         973ms   106.2MB   2267.1KB  118.8%
 44 mania    mania_mp4_2x.mp4                         success  1366x768       1994ms   143.4MB   3271.4KB  188.8%
 45 mania    mania_png_no_sv.png                      success  2262x6289       173ms    82.8MB     51.8KB   90.3%
 46 mania    mania_gif_no_sv_30fps.gif                success  972x472         900ms    30.6MB    419.9KB  105.9%
 47 mania    mania_mp4_no_sv_30fps.mp4                success  684x384        1448ms   106.1MB   2331.9KB  130.6%
```
