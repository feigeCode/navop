# FTP and FTPS remote files

Navop manages remote files over FTP and FTPS. FTP is a separate protocol from SSH: you can create a standalone FTP connection, or switch the remote-file protocol of an existing SSH connection record to FTP or FTPS. That reuses the connection name, workspace, and credential reference, but the underlying connection is always separate.

## Two ways to use it

- **Standalone FTP connection**: choose FTP when creating a connection and fill in host, port, and credentials. Use it for servers or network devices that only expose FTP.
- **Remote-file protocol on an SSH connection**: in the **Remote files** tab of the SSH/FTP connection form, choose SFTP, FTP, or FTPS. The SSH terminal always uses SSH, while the remote-file panel connects with the selected protocol, so one record covers both ways to reach the same host.

The protocol also affects how a double-click opens the connection: you can configure the default as the terminal or the dual-pane file view.

## FTPS and encryption

FTPS uses explicit AUTH TLS (`use_tls`). With it enabled, Navop negotiates TLS on the control connection before transferring data. Compared with SFTP, plain FTP sends the control and data connections unencrypted, so usernames, passwords, and file contents can be read by a man in the middle. Therefore:

- Prefer SFTP whenever it is available.
- If FTP is required, enable FTPS and verify the server certificate.
- Never reuse an FTP password on other systems.

## Connections and passive mode

FTP needs two links: a control connection and a data connection. Most servers use passive mode (PASV), where the client opens the data connection. If the connection is established but listing or transfer fails, a firewall, NAT, or security group is usually not allowing the passive port range the server reports. Switch passive mode or ask the administrator to adjust the port range.

Direct FTPS connections by IP address depend on correct certificate validation and AUTH TLS negotiation. If the server certificate does not match the host name, handle it according to the actual server setup after confirming it is safe.

## File operations

Once connected, the FTP remote-file panel shares the same file manager as SFTP: upload, download, rename, delete, create directories, view permissions, and the transfer queue. FTP servers may treat permissions, symbolic links, and hidden files differently from SFTP, so refresh the directory and verify file size and modification time after an operation.

## Troubleshooting order

1. Use **Test connection** to confirm host, port, and credentials.
2. If it connects but cannot list or transfer, check passive mode and the port range.
3. If the FTPS handshake fails, confirm the server supports explicit AUTH TLS and that the certificate is trusted.
4. If an upload succeeds but the service does not change, check the remote path, permissions, and whether the service needs a reload.
