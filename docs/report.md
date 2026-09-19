# 批量渲染报告

本文件汇总三类批量渲染任务的原始报告。每份报告均保留生成脚本输出的完整内容，便于对比不同渲染流程的耗时、资源占用和成功率。

## Rust 批处理报告

该部分对应 `batch-rust/report.txt`，记录 Rust 批量渲染 PNG 和 GIF（包括 Mod、转谱及时间点组合）的任务耗时、峰值内存、输出大小和 CPU 使用率。

```text
osu-beatmap-preview (Rust) 批量渲染报告
生成时间: 2026-09-19 15:46:03
任务总数: 90    成功: 90    失败: 0
总耗时: 55438ms (55.4s)    峰值内存(单进程最大): 590.7MB
CPU 统计: 平均 131.8%    最高 398.2%    总CPU时间 92.0s

  #  MODE    LABEL                                      STATUS        TIME   PEAKMEM        SIZE       %
--------------------------------------------------------------------------------------------------------------
  1  std     standard_738063.png                        success      827ms    73.5MB     409.7KB   45.3%
  2  std     standard_2875069.png                       success      347ms    73.6MB     328.3KB   90.1%
  3  std     standard_4897202.png                       success      310ms    73.6MB     375.5KB  100.8%
  4  std     standard_1024742.png                       success      325ms    74.4MB     434.4KB   76.9%
  5  std     standard_372245.png                        success      789ms    77.8MB    1605.9KB   93.1%
  6  std     standard_1529760.png                       success     2363ms   147.9MB       763KB   99.8%
  7  std     standard_5467386.png                       success      659ms      80MB     220.1KB   94.8%
  8  std     standard_738063.gif                        success     1005ms   111.5MB    2021.2KB  247.2%
  9  std     standard_2875069.gif                       success      939ms      98MB      1583KB  204.7%
 10  std     standard_4897202.gif                       success      979ms    90.5MB    2039.8KB  191.5%
 11  std     standard_1024742.gif                       success     1004ms     106MB    2154.7KB  224.1%
 12  std     standard_372245.gif                        success     1749ms   125.9MB    7685.7KB  340.4%
 13  std     standard_1529760.gif                       success     2900ms   590.7MB    3911.3KB  398.2%
 14  std     standard_5467386.gif                       success     1380ms   118.6MB    1201.6KB  289.9%
 15  taiko   taiko_4242023.png                          success      199ms    82.5MB     199.2KB   86.4%
 16  taiko   taiko_1418246.png                          success      386ms   120.1MB     486.2KB     85%
 17  taiko   taiko_4590053.png                          success      164ms    60.2MB     115.9KB   66.7%
 18  taiko   taiko_2923535.png                          success      293ms    85.4MB     358.9KB   90.7%
 19  taiko   taiko_5651058.png                          success      507ms   242.1MB     506.1KB   98.6%
 20  taiko   taiko_3726150.png                          success      257ms   116.1MB     201.4KB   91.2%
 21  taiko   taiko_4242023.gif                          success      355ms    28.1MB    1079.6KB  140.8%
 22  taiko   taiko_1418246.gif                          success      356ms    31.2MB    1365.1KB  131.7%
 23  taiko   taiko_4590053.gif                          success      319ms      28MB     823.1KB  122.5%
 24  taiko   taiko_2923535.gif                          success      414ms    32.6MB      2409KB  184.9%
 25  taiko   taiko_5651058.gif                          success      324ms    29.4MB      1158KB  115.7%
 26  taiko   taiko_3726150.gif                          success      292ms    27.8MB     787.9KB  133.8%
 27  catch   catch_3852338.png                          success      232ms    57.6MB     253.3KB   87.6%
 28  catch   catch_3807626.png                          success      445ms   156.2MB     716.4KB   91.3%
 29  catch   catch_944502.png                           success      225ms      44MB     556.3KB   69.4%
 30  catch   catch_2571609.png                          success      883ms   358.7MB    2329.5KB   93.8%
 31  catch   catch_265177.png                           success      383ms     102MB    1246.6KB   73.4%
 32  catch   catch_3852338.gif                          success      666ms      53MB     972.8KB  133.7%
 33  catch   catch_3807626.gif                          success      755ms    53.7MB    1104.7KB  171.8%
 34  catch   catch_944502.gif                           success      824ms    53.6MB    3828.1KB  144.1%
 35  catch   catch_2571609.gif                          success      726ms    65.2MB      1146KB    142%
 36  catch   catch_265177.gif                           success      850ms    53.7MB    3710.7KB  145.2%
 37  mania   mania_4312004.png                          success      480ms   226.7MB     256.1KB   78.1%
 38  mania   mania_4610729.png                          success      168ms    74.1MB      57.6KB   74.4%
 39  mania   mania_5061439.png                          success      134ms    60.8MB      67.4KB   81.6%
 40  mania   mania_4789195.png                          success      354ms   145.3MB     379.6KB   92.7%
 41  mania   mania_3793380.png                          success      259ms   118.4MB     229.3KB   66.4%
 42  mania   mania_4665942.png                          success      189ms    98.3MB      81.1KB   82.7%
 43  mania   mania_5354177.png                          success      260ms   127.2MB      96.1KB   84.1%
 44  mania   mania_5221843.png                          success      137ms    46.7MB     102.3KB     57%
 45  mania   mania_5369780.png                          success      289ms   127.5MB      91.5KB   86.5%
 46  mania   mania_4972672.png                          success      356ms   193.9MB     149.5KB   92.2%
 47  mania   mania_5013742.png                          success      319ms   182.4MB      84.9KB   88.2%
 48  mania   mania_4312004.gif                          success      665ms    31.2MB    1117.8KB  110.4%
 49  mania   mania_4610729.gif                          success      588ms    29.8MB     375.3KB  106.3%
 50  mania   mania_5061439.gif                          success      599ms    29.5MB     582.3KB  122.6%
 51  mania   mania_4789195.gif                          success      634ms      35MB     813.6KB  145.4%
 52  mania   mania_3793380.gif                          success      512ms    32.6MB     531.4KB  100.7%
 53  mania   mania_4665942.gif                          success      663ms    31.5MB     756.8KB  157.9%
 54  mania   mania_5354177.gif                          success      575ms    30.6MB     303.4KB   92.4%
 55  mania   mania_5221843.gif                          success      724ms    35.5MB       922KB  107.9%
 56  mania   mania_5369780.gif                          success      697ms    33.4MB     390.4KB  123.3%
 57  mania   mania_4972672.gif                          success      847ms    39.3MB     613.9KB    131%
 58  mania   mania_5013742.gif                          success     1131ms      52MB       344KB   95.3%
 59  std     standard_738063_hd-hr.gif                  success      879ms    96.8MB     942.3KB    208%
 60  std     standard_2875069_hr.png                    success      292ms    72.9MB     204.3KB   96.3%
 61  std     standard_4897202_dt1.3.gif                 success     1038ms    94.9MB    2014.9KB  188.2%
 62  std     standard_1024742_daar9.5-dacs4.5.gif       success     1109ms   104.3MB    2091.6KB  232.5%
 63  std     standard_5467386_ez-hd.gif                 success     1851ms   158.6MB    1859.2KB  319.9%
 64  taiko   taiko_4242023_hr.gif                       success      292ms    27.6MB     670.6KB  160.5%
 65  taiko   taiko_1418246_dt.gif                       success      347ms    31.3MB    1091.8KB  139.6%
 66  taiko   taiko_4590053_sw.png                       success      137ms      60MB     117.3KB   91.2%
 67  taiko   taiko_2923535_cs.gif                       success      411ms    33.7MB    2273.6KB  159.7%
 68  catch   catch_3852338_hr.gif                       success      660ms    52.7MB     657.6KB  168.1%
 69  catch   catch_3807626_ez.png                       success      356ms    86.5MB     737.2KB  109.7%
 70  catch   catch_944502_dt1.4.gif                     success      991ms    53.7MB    3815.6KB  159.2%
 71  mania   mania_4312004_in.png                       success      546ms   226.7MB     267.8KB   91.6%
 72  mania   mania_4610729_ho.gif                       success      607ms    29.7MB     375.3KB  115.8%
 73  mania   mania_5061439_cs.gif                       success      603ms    29.2MB     539.8KB  134.7%
 74  convert mania_5473947_convert_ds.gif               success     1072ms    48.5MB     628.4KB  100.6%
 75  convert taiko_738063_convert.png                   success      134ms    54.6MB        74KB  104.9%
 76  convert taiko_2875069_convert.gif                  success      296ms    28.8MB       670KB  153.1%
 77  convert catch_4897202_convert.png                  success      294ms   121.7MB       500KB     85%
 78  convert catch_1024742_convert.gif                  success      665ms    54.1MB    1094.3KB  152.7%
 79  convert mania_372245_convert.png                   success      198ms    73.2MB     109.8KB   86.8%
 80  convert mania_1529760_convert.gif                  success      290ms    33.4MB      66.6KB  118.5%
 81  convert taiko_5467386_convert.gif                  success      294ms    32.4MB     186.2KB  170.1%
 82  convert taiko_260177_convert.png                   success      168ms    61.6MB        88KB   65.1%
 83  convert catch_260177_convert.png                   success      761ms   148.5MB     408.1KB   96.5%
 84  std     standard_738063_time-points30-40-50-60.gif success      973ms   112.8MB    2002.1KB  247.3%
 85  std     standard_2875069_time-points10-25-60.gif   success      882ms    96.8MB    1615.1KB  207.3%
 86  std     standard_4897202_time-points45.gif         success      915ms    89.3MB    2013.5KB  174.2%
 87  convert mania_738063_convert_in.gif                success      665ms    33.3MB       717KB   91.6%
 88  convert catch_2875069_convert_hr.gif               success      590ms      54MB     537.6KB  140.4%
 89  std     standard_4897202_hd-dt1.25_time-points20-40.gif success      851ms    94.2MB    1001.9KB  229.5%
 90  convert taiko_5467386_convert_hr_time-points15-30.gif success      260ms    32.3MB     120.6KB   90.1%
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
