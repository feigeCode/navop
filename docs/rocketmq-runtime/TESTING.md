# rocketmq-runtime 真实环境联调清单

本 crate 的单元测试(帧编解码/DTO serde/ACL 签名/commitlog 解析/mock TcpServer)不依赖
真实集群即可全绿。本文档描述如何在本地以 Docker 启动 RocketMQ 并对
`RocketmqConnection`(`MiddlewareAdmin` 实现)做冒烟联调。

## 一、启动本地 RocketMQ 集群(4.x / 5.x 各一套)

### 4.9.x(验证 4.x 兼容)

```bash
# NameServer
docker run -d --name rmqnamesrv \
  -p 9876:9876 \
  apache/rocketmq:4.9.4 sh mqnamesrv

# Broker(需指定 namesrv 地址)
docker run -d --name rmqbroker \
  --link rmqnamesrv:namesrv \
  -p 10911:10911 -p 10909:10909 \
  -e "NAMESRV_ADDR=namesrv:9876" \
  apache/rocketmq:4.9.4 sh mqbroker \
  -c /home/rocketmq/rocketmq-4.9.4/conf/broker.conf \
  -n namesrv:9876
```

> 注意:容器内 Broker 注册的地址是容器内网 IP,宿主机直连时需要在 `broker.conf`
> 追加 `brokerIP1=127.0.0.1` 后重启 Broker,否则路由返回的地址不可达。

### 5.x(验证 5.x 兼容)

```bash
docker run -d --name rmqnamesrv5 -p 9877:9876 apache/rocketmq:5.3.2 sh mqnamesrv
docker run -d --name rmqbroker5 \
  --link rmqnamesrv5:namesrv \
  -p 10912:10911 -p 10910:10909 \
  -e "NAMESRV_ADDR=namesrv:9876" \
  -e "brokerIP1=127.0.0.1" \
  apache/rocketmq:5.3.2 sh mqbroker -n namesrv:9876
```

## 二、CLI 工具预置数据(便于核对)

```bash
# 进入 broker 容器执行
docker exec -it rmqbroker sh -c "\
  mqadmin updateTopic -n namesrv:9876 -b 127.0.0.1:10911 -t order-topic -r 8 -w 8 -p 6 && \
  mqadmin updateSubGroup -n namesrv:9876 -b 127.0.0.1:10911 -g order-group && \
  export NAMESRV_ADDR=namesrv:9876 && \
  sh /home/rocketmq/rocketmq-4.9.4/bin/tools.sh \
     org.apache.rocketmq.example.quickstart.Producer"
```

## 三、冒烟步骤(以本 crate 的测试形态)

`RocketmqConnection` 的冒烟可通过一个临时集成测试或示例驱动,
核心调用序列(均应返回 `Ok`):

| 步骤 | 调用 | 断言要点 |
| --- | --- | --- |
| 1 | `RocketmqConnection::new(params)` + `test_connection()` | `GET_BROKER_CLUSTER_INFO(106)` 收到响应 |
| 2 | `cluster_overview()` | 集群名 `DefaultCluster`、Broker master 地址可达 |
| 3 | `list_topics()` | 含 `order-topic`/`TBW102`/`SCHEDULE_TOPIC_*`,`%RETRY%` 标记 RETRY |
| 4 | `create_topic(CreateTopicRequest{topic:"smoke-topic",queue_count:4,perm:"6",..})` | 再次 `list_topics()` 出现且 queue_count=4 |
| 5 | `send_message(topic,"tagA","key-1",body)` | `status=="OK"`,记录返回 `message_id` |
| 6 | `query_messages(ByKey{topic,key:"key-1"})` | 命中 ≥1 条,body_text 与发送一致 |
| 7 | `message_detail(topic, message_id)` | 与发送的 tag/key/时间吻合 |
| 8 | `query_messages(ByTimeWindow{begin=发送前,end=now,page:1,page_size:20})` | 至少含刚发送的消息 |
| 9 | `list_groups()` | 含 `order-group`(无在线客户端时 client_count=0/None) |
| 10 | `group_detail("order-group")` | 无消费时 diff == broker_offset |
| 11 | `update_topic`(queue_count=6) → `topic_detail(topic)` | 队列数变化生效 |
| 12 | `delete_topic("smoke-topic")` | `list_topics()` 不再出现 |
| 13 | `metrics_snapshot()` | topic_count/tps_in/extras.broker_count 合理 |

### 断连/故障切换验证

- 停掉首个 NameServer(多 namesrv 配置 `a;b`),重复步骤 2:应自动切换到第二地址;
- 杀掉 Broker 后再启动,重复步骤 3:通道应自动重连(缓存通道失效重建)。

### ACL(4.x acl)验证

1. Broker 开启 ACL:`broker.conf` 追加 `aclEnable=true`,并按官方文档配置
   `conf/plain_acl.yml`(如 accessKey=rocketmq / secretKey=12345678);
2. 连接参数填 `access_key`/`secret_key` 后重复步骤 1-8;
3. 错误的 secretKey 应得到 `MiddlewareError::Auth`(服务端 NO_PERMISSION=16)。

### SSH 隧道验证

在连接参数中启用 `ssh_tunnel`(SSH 主机可连通 Docker 宿主机),重复步骤 1-3;
隧道场景下 NameServer/Broker 均经本地端口前置转发。

## 四、已知边界

- `VIEW_MESSAGE_BY_ID` 依赖消息 ID 中的存储地址与路由一致;消息被删除后查询报
  `消息不存在`;
- 时间窗口查询按"逐队列 SEARCH_OFFSET + QUERY_CONSUME_QUEUE 遍历 + 逐条 VIEW 补全"
  实现,大窗口多队列时有明显请求放大,建议 UI 层限制窗口跨度;
- 4.x 服务端 Topic 无类型信息,统一标记 `UNSPECIFIED`;5.x 读取 `+type` 属性。
