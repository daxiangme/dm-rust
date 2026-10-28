仅供测试的自签证书（不要用于生产）：`ca.pem` 签发了 `server-cert.pem`（SAN: localhost, 127.0.0.1）；`other-ca.pem` 是无关 CA，用于负例。
服务端配置：把 `server-cert.pem`、`server-key.pem` 拷到 `$DM_HOME/bin/server_ssl/`，`ca.pem` 拷为 `ca-cert.pem`，dm.ini 设 `ENABLE_ENCRYPT = 1`，重启实例。
