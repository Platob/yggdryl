# Native financial observations

Excel16 build20430 accepted382 cases across both epochs; both guarded runs passed
and restored the attached application. The fixture retains the input hashes,
raw COM values, error classification and saved XML caches. Observations are from
before and after SaveAs in the same owned workbook, not a reopen.

The first Rust slice replays only PV/FV:176 observations, each as wire and saved
formula text. No PMT/NPV equality is claimed by that slice. Their original answers
remain available while a separate answer-free operation-order probe settles
their kernels. Nonzero type means payments at the beginning, including0.5/-1.

PV/FV share growth and annuity factors. Integral powers use repeated squaring;
the annuity is ((growth-1)/rate)*(1+rate*due), before multiplication by payment.
Zero rate uses nper directly. Public Excel error/zero policy stays in its adapter.

Microsoft's algebra and argument contract:
https://support.microsoft.com/en-us/excel/functions/pv-function
https://support.microsoft.com/en-us/excel/functions/fv-function
https://support.microsoft.com/en-us/excel/functions/pmt-function
https://support.microsoft.com/en-us/excel/functions/npv-function
Native error/origin observations take precedence over generic prose where they
disagree; the NPV documentation's statement about reference errors is broader
than this Excel build's observed propagated #DIV/0!.
