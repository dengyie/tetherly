# Security

Do not file public issues that contain SMS bodies, one-time codes, pairing PINs,
clipboard contents, identity secret keys, or Noise static keys.

Crash reports must not include notification payloads. There is no telemetry.

Report vulnerabilities privately to the maintainers. Include:

- affected version / commit
- reproduction without real SMS contents (synthetic fixtures only)
- impact (pairing bypass, OTP leakage, remote code execution, etc.)

`TETHERLY_INSECURE_LOG=1` is debug-profile only and prints a startup warning.
Never enable it against real devices.
