# ch3 注册新的订阅者
## ch3.1 前期的准备工作
在开始一个项目时首选确定
1. web框架的选择
2. 数据库的选择
3. 数据库管理包的选择
4. 查询语句
## ch3.2 
web框架这里我们选择actix-web
## ch3.3 实现/health_check端点
/health_check端点是用来判断后端服务是否正常运行的端点。获得一个GET请求，如果正常返回200的状态码。
### ch3.3.1 使用actix-web编写代码
额外补充：我们可以使用cargo add命令来添加依赖包，而不是一定需要使用修改Cargo.toml文件的方法。
该命令包含在cargo-edit中，通过cargo install cargo-edit方法获取(对于cargo命令的扩展的获取方法都类似于这样)。
### ch3.3.2 actix-web应用程序刨析
#### ch3.3.2.1 服务器-HttpServer
HttpServer负责应用程序从哪里监听数据，允许的最大并发连接数，每秒有多少个新链接。也就是使用传输层提供的服务。
与客户端建立链接。
#### ch3.3.2.2 应用程序-App
App是整个应用程序的逻辑所在，包括路由，中间件和请求处理器。App是一个组件，接受请求作为输入，并返回响应作为输出。
route方法对应一个route调用。每一个端点都会有route调用。
#### ch3.3.2.3 端点-Route
通过route方法添加一个新的端点。
route方法接受两个参数
path: 一个字符串，用来接受动态的请求路径。
route: 一个Route结构体实例。
Route实例包含一个处理器和一组守卫。
多个守卫将满足条件的请求按照route的顺序分别与处理器进行匹配。
web::get() 实际上是Route::new().guard(guard::Get())
表示当且仅当请求的方式是Get时，才会将请求传入到处理器。
当一个新的请求到来时，App遍历所有的注册的端点，直到路径模板和守卫条件都完全匹配，交给处理器。
在greet函数当中指定返回类型为impl Responder的任何类型。
任何实现了Responder trait的类型都可以被转换为HttpResponder。
#### ch3.3.2.4 运行时--tokio
cargo expand用来展开宏。
像tokio::main这样的过程宏，接受一些token，然后按照指定的规则替换，得到新的token。
或者说按照指定的规则进行代码的替换和生成。
cargo expand依赖nightly编译器进行编译。
### ch3.3.3 实现健康检查处理器
actix-web对于端点传入的执行器的传入参数具有广泛的实用性，并不限制。
## ch3.4 第一次集成测试
测试应该是自动话的：每次提交的变更时，这些检查都应该在持续集成流水线中运行。
### ch3.4.1 如何对端点进行测试
API是达到目标的手段，是一种向外部世界公开用于执行某种任务的工具。
测试API通常使用黑盒测试。
我们不仅要测试执行器的执行结果，也要检查是否通过指定的端点到达了对应的执行器。
我们选择完全黑盒的方案，使用客户端与其交流。
### ch3.4.2 应该将测试放在哪里？
嵌入式测试模块作为单元测试。
tests文件夹中的内容作为集成测试。
/tests中的任何东西都会被编译为二进制文件。
### ch3.4.3 改变项目结构以便用于测试
/tests下的所有测试代码都会编译为独立的二进制文件
所有的测试代码都应该通过包的形式被导入。但是当前的项目式二进制形式的，也就是二进制包，二进制包是不可导入的。
main.rs是与package.name同名的二进制包的根文件，也就是入口文件。
我们需要自己去指定lib库
## ch3.5 实现第一个集成测试
### ch3.5.1 优化
#### ch3.5.1.1 清理资源
当tokio运行时关闭时，其上的所有任务都会关闭，不管是否完成。
tokio运行时要想结束，只要顶层任务结束就结束，也就是初始化运行时时传入的future形成的任务。
tokio::test的每一个测试用例开始时都会启动一个新的运行时。
#### ch3.5.1.2 随机选择一个端口
spawn_app()每次都选择在 8000 端口上运行程序
1. 如果我们的自己的二进制程序要运行，测试失败
2. 并发运行测试时，失败。

我们将run函数接受一个参数，用来指定地址。
如何为测试找到随机的一个可用端口呢？
操作系统提供支持：端口0
尝试绑定端口0将出发操作系统扫描可用端口，将其绑定到应用程序。
但是这样端口实在运行时确定的，我们测试时不知道端口是什么。需要修改spawn_app()返回。
1. std::net::TcpListener
使用TcpListener监听某一个端口，然后通过listen交给HttpServer
TcpListener:local_addr返回一个SocketAddr，通过.port()绑定的实际端口

## ch3.7 处理HTML表单
### ch3.7.1 提炼需求
我们需要从访问者获取哪些信息来使他们注册为订阅者。
需要电子邮件地址和所有新订阅者的名字。
使用HTML表单时，application/x-www-from-urlencoded是最合适的编码方式。
键和值以键值对的方式编码(元组)。元组之间用&分割，键和值用=分割。
键和值中的非字母数字字符按百分号编码。
1. 如果使用该格式提供了一对有效名字和电子邮件地址，返回200 Ok
2. 如果缺失任何一个字段，返回400 BAD REQUEST
### ch3.7.2 以测试的方式捕获需求
驱动测试开发，也叫参数化测试。
### ch3.7.3 从POST请求中解析表单数据
404 NOT FOUND请求了不存在的内容，也就是访问了不存在的端点.
#### ch3.7.3.1 提取器
提取器从传入的请求中解析出特定的部分。
1. Path: 用于从请求路径获得动态路径参数
2. Query: 用于获取查询参数
3. Json 用于解析JSON编码的请求体
4. Form: 用于从请求体中获取url编码的数据，或者发送URL编码的数据进行响应
#### ch3.7.3.2 Form和FromRequest
````
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct Form<T>(pub T)
````
Form只是一个包装器。
提取器是一个实现了FromRequest trait的类型。
````
pub trait FromRequest: Sized {
    type Error = Into<actix_web::Error>;
    
    async fn from_request(
        req: &HttpRequest,
        payload: &mut Payload,
    ) -> Result<Self, Self::Error>
}
````
路由处理器函数中的所有参数都必须实现FromRequest trait， actix-web将为每一个参数调用from_request, 如果提取成功，则会执行真正的执行器函数。
如果有一个提取失败，则会将相应的错误返回。
UrlEncoded透明的处理的压缩和未压缩的有效载荷，处理了请求体字节流分块到达的事实。
#### 3.7.3.3 Rust中的序列化
1. 泛型
serde本事不提供对任何特定数据格式的序列化和反序列化
serde实现了一组接口或者数据模型，如果要实现一个支持新数据格式的序列化库，需要实现Serializer trait， 数据模型有29种（对应的是格式）。
Serializer 实现指定了每种类型每种类型如何映射到特定的数据格式。
Serialize trait，应该指定如何根据serde的数据模型使用Serializer特质对其进行分解
2. 效率
单态化
serde: 中间数据模型是通过trait方法隐式定义的，没有真正的中间序列化结构。
3. 便捷
#[derive(Serialize)] #[derive(Deserialize)]过程宏自动为类型定义生成适当的实现。
#### 3.7.3.4 整合一切
处理器接受到请求，actix-web调用Form<FormData>的from_quest方法，调用serde_urlencoded和 FormData 的Deserialize实现将请求反反序列化为FormData
## ch3.8 存储数据： 数据库
### ch3.8.1 选择数据库
最后选择关系数据库: PostgreSQL
### ch3.8.2 选择数据库包
tokio-postgre 不支持安全编译 SQL查询 支持异步
sqlx          支持         SQL查询 支持异步
diesel        支持         DSL查询 不支持
### ch3.8.3 带有副作用的集成测试
也就是要测试是否持久化存储。
我们可以实现端点 GET /subscriptions, 但是担心安全性。
再测试用例中编写一个简单低的查询
### ch3.8.4 数据库初始化
#### ch3.8.4.1 Docker
#### 数据库迁移
向数据库中添加新表，需要更改数据库模式--也叫做数据库迁移
1. sqlx-cli 用于管理数据库迁移。可以通过cargo install 安装
2. 创建数据库，sql database create 但是启动数据库实例时已经自带了叫做newsletter的默认数据库。
但是在生成环境中，我们还是要创建。依赖DATABASE_URL环境变量创建。
DATABASE_URL格式如下
postgres://${DB_USER}:${DB_PASSWORD}@{DB_HOST}@localhost:${DB_PORT}/
执行sqlx database create命令时postgres无法链接，我们需要等待Postgres完成操作，然后开始对其运行命令。
3. 添加迁移
sqlx migrate add create_subscriptions_table
创建了一个migrations文件夹，里面存储的是迁移文件，每一个迁移文件记录的是数据状态的改变，表的变更，数据变化的SQL语句
主键使用没有任何业务含义的合成主键例如UUID
数据库约束会影响写入的吞吐量，因为有检查操作。
4. 运行迁移
使用sqlx migrate run 进行迁移
得到一个subscriptions表和一个_sqlx_migrations表，后续表记录的是sqlx对数据库进行了哪些迁移。
### ch3.8.5 编写第一个查询
#### ch3.8.5.1 sqlx功能标志
runtime-actix-rustls 使用actix运行时作为功能的一部分，使用rustls作为TLS后端
macros 允许使用query! 和 query_as!
postgres 解锁了Postgres 特定功能。
uuid 添加了将SQL UUID映射到uuid包中的Uuid
chrono 用来处理 SQL timestamptz 处理
#### ch3.8.5.2 配置管理
使用PgConnection提供connect方法用来链接数据库。
使用配置管理机制来管理我们的配置。config包可以用来管理和处理配置。
#### ch3.8.5.3 链接Postgres
PgConnection::connect 接受单个链接字符串作为输入。也就是DATABASE_URL。
sqlx在编译时与postgres进行互动，以检验查询是否合法，依赖于DATABASE_URL环境变量确定数据库的位置。
sqlx将从.env文件中读取DATABASE_URL。
.env 与开发过程，构建和测试步骤有关。configuration.yaml用在编译后更改应用程序的运行时行为。
#### ch3.8.5.5 升级持续集成流水线
由于sqlx的编译时检查需要一个正在运行的数据库。而CI/CD也包含编译时检查，所以我们需要更新CI/CD
## ch3.9 持久化一个新的订阅者
### ch3.9.1 actix-web中的应用程序状态
目前我们的应用程序是无状态的：处理器只处理来自请求的数据。
应用程序状态：将单个请求声明周期无关的数据附加到应用程序上。也就是整个App共享的数据。
HttpServer期望PgConnection是克隆的，为什么？
### ch3.9.2 actix-web工作流程
HttpServer接受一个返回App的闭包。这是因为actix-web的运行时模型：actix-web为机器上的每一个可用的核心创建启动一个工作进程。
每一个工作进程都有自己的App副本，该副本通过HttpServer调用闭包得到。这就是为什么要求是可克隆的。
由于PgConnection是一个系统资源不可克隆。我们可以借助提取器web::Data。web::Data将链接包装到Arc中。每一个App副本或者Arc的克隆。
### ch3.9.3 Data提取器
Data提取器从哪里获取了PgConnection呢？from_request不是接受HttpRequest和Payload吗？
actix-web使用哦一个类型映射来表示其应用程序状态：HashMap, 可以将任意类型存储到它们唯一的类型标识符(TypeId::of)。
当一个新的请求到来是，web::Data查询函数签名中指定类型的TypeId,如果查询到了，将存储的值Any进行强制类型转化
### ch3.9.4 INSERT语句
sqlx::query!().execute()函数接受一个实现了sqlx的Executor trait的参数。
&PgConnection没有实现，而&mut PgConnection实现了。
sqlx不允许在同一个数据库连接上同时运行多个查询。 
PgPool的共享引用也是新了Executor。
PgPool是一个Postgres数据库连接池。当对&PgPool进行查询时，sqlx将从连接池中借用PgConnection并使用他进行查询，如果没有可用的查询。创建一个新链接或者等待查询。
## ch3.10 更新测试
### ch3.10.1 测试隔离
现在的数据库是一个庞大的全局变量：所有的测试都在和他交互，而他保留下的结果将被用于其他测试组件。
500 INTERNAL Server Error 服务器内部处理出现错误。
为了避免数据库持久化数据后。造成不同测试的测试结果依赖的数据库状态出现冲突。
以及某一个测试不具有幂等性。
我们需要将测试隔离：也就是确保各个测试访问数据库时，不会因为执行的次数，顺序导致出现意料之外的错误。
1. 将整个测试包装为SQL事务，然后回滚
2. 每一个集成测试都有一个单独的逻辑数据库
第一种方法适合于单元测试，因为回滚一个事务快于启动一个新的数据库。但是在集成测试中很麻烦？
第二种方法虽然慢，但是实现简单。
