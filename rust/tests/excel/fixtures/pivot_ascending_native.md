# Excel-authored explicit ascending PivotTable fixture

Input: `rust/target/excel-desktop/p6-native-ascending-control-v1/pivot-native.xlsx`, SHA-256 `0b80eb726120e3e0b366e9d62e85ec02fc866ea9cd534f245d9db25526d534ea`.
Report: `rust/target/excel-desktop/p6-native-ascending-control-v1/results.json`, SHA-256 `10c871cc21f451d7a84173e75531983f7a93b9e27dd25029e5b93fc53984d55f`.
Excel 16.0 build 20430.0, UI LCID 1036; execution passed and cleanup completed. All five pivot axes read back as `SortOrder=1` and saved with `sortType=ascending`; root data/grand captions are authored English. The original `pivot_excel.xlsx` remains the immutable native manual-default input for refusal and inventory tests.
