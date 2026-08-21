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
# ch4 遥测
## ch4.1 未知的未知
测试套件不能证明代码完全正确。我们还需要探索其他验证正确性的途径。
运行时环境是极为复杂的，我们可能没有考虑到。
1. 如果应用程序与数据库断开链接时会发生什么？
2. 如果有攻击者尝试构造POST /subscriptions 请求，并发送恶意数据，能够合理处理吗？
未知的未知问题：从未处理过从未预见过的问题。
## ch4.2 可观测性
遥测数据：由应用程序自动收集的运行时的信息。
面对“未知的未知”问题，我们不知道何时发生，需要哪些信息才能发现问题。
所以我们需要构建一个可观测的应用程序。
可观测性的目标是可以回答对环境提出的任何问题，无需提前知道问题是什么。
构建可观测的应用程序：
1. 利用插桩收集高质量的遥测数据
2. 利用工具切分和处理所有收集的数据，用来回答需要了解的问题。
## ch4.3 日志
日志是最基本的遥测数据。
### ch4.3.1 log包
log包用来做日志记录，包含5个宏：trace，debug，info，warn和error。
提供的功能是用宏的名称所代表的日志级别，记录一条日志。
trace最低级别，日志比较详细。接下来分别是debug, info, warn, error.
error是最高级别，用于记录严重到影响到用户体验的错误。
### ch4.3.2 actix-web的Logger中间件
中间件指的是HTTP请求到服务器端处理时之间，以及处理完毕之后返回到客户端之间的部分。
actix-web提供了一个Logger中间件，每一个HTTP请求都会产生一个日志。
### ch4.3.3 外观模式
log包通过外观模式处理日志记录的处理.
它提供记录日志的接口，但是没有限定处理这些日志的方法。
在main函数的起始位置，调用set_logger函数，传入实现了Log trait的实现：每当一个日志通过Log::log被记录时，都会调用该实现。
如果没有调用，所有日志都会被丢弃。
现在初始化记录器。
env_logger::Logger将日志输出到控制台，输出格式 
 \[<时间戳> <日志级别> <模块路径>] <日志消息>
RUST_LOG环境变量来决定输出和过滤哪些日志。
RUST_LOG = debug cargo run将所有debug以上级别的日志记录，包括应用程序本身的日志，使用的包中的日志。
如果设置为RUST_LOG=zero2prod 则会过滤所有的使用的包的日志。

## ch4.4 插桩POST /subscriptions
使用log包来插桩POST /subscriptions 插桩也就是收集遥测数据。
### ch4.4.1 与外部系统的交互
经验法则：在所有通过网络和外部系统交互的过程中，都要反复不断地记录当前状态。
### ch4.4.2 像用户一样思考
为了更方便了解决未知的未知，我们需要站方位词的角度提出一个问题，然后尝试丰富日志消息，方便我们找到原因。
### ch4.4.3 日志应该便于关联
各个请求对应的日志消息应该存在明确的分离点。这样易于分析。
我们需要一种方法将日志和请求绑定在一起。
使用请求id即可：开始处理一个请求时，首先生成一个随机的标识符，用于将日志和请求关联起来。
由于request-id是在subscribe函数中生成的，所以Logger中间件是知道该id的，所以我们也无法知道该请求对应的返回的状态码是什么。因为所有Logger记录的日志没有request-id。
## ch4.5 结构化日志
确保所有的日志记录都有一个请求对应的关联ID。
### ch4.5.1 tracing包
tracing允许包和应用程序记录结构化事件，并在其中包含用于说明结构性和因果性的信息，以此开展日志形式的诊断。
### ch4.5.2 从log迁移到tracing
直接将log替换为tracing，启用log功能标志，当tracing的宏记录了一个事件或者跨度，都会被log收集起来。
所以输出日志暂时没有区别。
### ch4.5.3 tracing中的跨度
跨度可以根据程序结构更好地捕获信息。我们想要创建一个跨度，其与当前所处理的请求相对应。
info_span!宏创建一个跨度，我们可以使用结构化信息以键值对的方式存储起来。
可以显示的给出键名。使用%修饰变量，表示log日志记录时使用其std::fmt::Display来实现。
我们需要显示的使用enter函数来进入跨度。
.enter()得到一个Enter类型，这是一个守卫对象。这个变量在析构之前，之后的所有下游跨度都会被注册为子跨度。
。
创建跨度时输出传入跨度的第一个形参。
进入当前跨度 ->
退出跨度 <- (Enter被析构)
关闭跨度 -- (跨度本身被析构)

### ch4.5.4 插桩Future
让跨度模拟一个future的生命周期：当future被轮询时，进入所对应的跨度；当future被挂起时，退出对应的跨度。
使用Instrument扩展future即可。
Instrument::instrument ： 以跨度为参数，每当self，也就是future被轮询时，进入该跨度。future被挂起时，退出该跨度。
### ch4.5.5 tracing的Subscriber
由于evn_logger无法解析tracing中跨度的结构化数据。
需要用tracing替换掉env_logger即可。也就是使用Subscriber即可。
### ch4.5.6 tracing-subscriber
使用tracing-subscriber，该包实现了trace::Subscriber trait。
tracing-subscriber提供了一个trait Layer。使得跨度数据能够以流水线的方式处理。
Registry实现了Subscriber trait，并处理架构中最复杂的内容。
### ch4.5.7 tracing-bunyan-formatter
1. tracing_subscriber::filter::EnvFilter 可以根据跨度的级别和来源来筛选跨度
2. tracing_bunyun_formatter::JsonStorageLayer 可以处理跨度数据，将其转换为易于处理的Json数据。并发给下游的层次，能将上游跨度的上下文传播到下游
3. tracing_bunyun_formatter::BunyanFormatterLayer: 在JsonStorageLayer的基础之上工作，以兼容bunyan的JSON格数输出
### ch4.5.8 tracing-log
actix-web的日志去哪呢？
tracing的log功能标志确保每当tracing事件发生时都会发出一条日志记录，log的记录器可以将其收集起来。反过来则不成立。log日志自身不会在记录时发送tracing消息。
我们可以使用tracing-log包中的LogTrace来解决。
### ch4.5.9 删掉未使用的依赖
使用cargo-udeps 去除无用的依赖。
cargo-udeps使用nightly编译，输出的是不需要的依赖的名称
### ch4.5.10 清理初始化流程
### ch4.5.11 集成测试中的日志
在测试套件中使用结构化日志，可以大幅度提高我们调试的效率。
我们不希望每个测试套件都阅读大量的日志记录，也就是尝试忽略测试。
对于print/println 可以使用cargo test -- --nocapture实现。
对于tracing需要在get_subscriber额外加入一个参数。用于控制日志是否应该被输出。
### ch4.5.12 清理插桩代码
日志的收集为函数的实现带来了干扰，我们希望将函数的所有步骤都在span的上下文中，也就是将函数包装在span中。
我们可以通过使用tracing::instrument过程宏实现。
#[tracing::instrument]在函数声明处创建了一个跨度，并将所有的参数传入到跨度的上下文中。我们可以通过skip指令忽略。
name用于给出函数跨度自身的日志信息。
我们也可以使用fields指令添加部分值进入上下文。
### ch4.5.13 保护隐私 secrecy
tracing::instrument会默认将传入函数的参数添加到上下文，这会使得我们输出的日志包含参数，即使他不应该被输出。所以我们可以引入secrecy::Secret来避免这个问题。
他会显式地将某个字段标记为敏感信息。
Secret是一个包装器，要访问其内部的内容使用Secret提供的expose_secret()方法。
secret的Debug实现会输出`Secret([REDACTED STRING])`。
这个类型还可以充当文档的类型，说明哪些类型是隐私的。
### ch4.5.14 请求ID
如何确保处理相同请求的过程中收集到的所有日志，包括状态码，都关联了request_id.
1. 如果不去改动actix-web::Logger中间件，可以尝试去添加另外一个中间件RequestIdMiddleware
其可以生成唯一的请求Id，创建一个新的跨度，上下文包含了请求ID，将下游的中间件都包装到这个新创建的跨度中。
2. 使用tracing生态系统中的工具。tracing-actix-web包
# ch5 上线
我们将学习如何将Rust程序打包为Docker镜像，并将其部署到DigitalOcean的应用平台。
并得到一个持续集成流水线：每次向main分支提交时，都会自动触发部署最新版本的应用程序。
## ch5.1 我们必须讨论部署问题。
将最新的应用程序部署是非常重要的。
## ch5.2 选择工具
### ch5.2.1 虚拟化： Docker
本地开发环境和生产环境是截然不同的。
把源代码直接部署到生产服务器上是不够的。我们的软件可能依赖于底层操作系统的能力。
确保软件正确执行的方法就是严格控制他的运行环境：
代码不直接部署到生产环境中，而是被部署到一个包含应用程序的独立环境中。
### ch5.2.2 托管：DigitalOcean
托管平台。
## ch5.3 应用程序的Dockerfile
编写一个Dockerfile，用来构建和执行应用程序作为Docker容器。（Docker容器是一个虚拟化：包含一个独立的环境，将我们的应用程序部署在上面）
### ch5.3.1 Dockerfile
Dockerfile是应用程序环境的模板。
Dockerfile是分层组织的：从一个基础镜像（通常是一个富含编程语言工具链的操作系统）开始，然后执行一系列命令。逐步构建所需要的环境。
根据模板创建一个名为'zero2prod'的Docker镜像。
### ch5.3.2 构建上下文
Docker build生成一个从Dockerfile和构建上下文开始的镜像。Docker镜像是一个包含应用程序，依赖工具链的分层文件系统快照。（也可以说是一个小型的操作系统）
构建上下文决定了主机上的哪些文件在Docker容器中对COPY可见。
使用'.'告诉Docker将当前目录作为镜像的构建环境。
### ch5.3.3 sqlx离线模式
由于sqlx在编译时需要建立和.env 中DATABASE_URL所指定的数据库的链接。但是如果没能连接上，就会发生错误。
解决方法：
1. 构建时使用 --network 参数让镜像和运行在本机的数据库相连接。
2. 由于不同操作系统中Docker网络实现的差异，我们可以使用新引入的sqlx离线模式。
sqlx prepare执行的与调用cargo build类似，都是将查询的结果保存在一个元数据文件，然后有sqlx自行检查，可以跳过查询，从而实现离线构建。
使用cargo sqlx prepare -- --lib执行
如何确保sqlx-data.json是最新的呢？可以在持续集成流水线中使用--check标志
### ch5.3.4 运行镜像
构建镜像时的--tag 是给镜像打上的标签。
我们可以使用这个标签在其他命令中引用这个镜像。特别实在运行时。
docker run zero2prod。
docker run 将触发dockerfile中ENTRYPOINT中的命令的执行。
PgPool::connect_laze()推迟链接在启动时建立--只会在首次尝试使用连接池时尝试建立链接。
### ch5.3.5 网络
在默认情况下，Docker镜像不会把端口暴露给宿主机。我们需要明确的使用 -p 命令来显示端口。
我们使用127.0.0.1作为主机地址->指示应用程序只接受来自同一台机器的链接。
我们可以使用0.0.0.0作为主机地址，此时应用程序可以接受任何网络接口来的链接。
为了避免安全性问题，我们将继续使用"127.0.0.1"进行本地开发，Docker镜像中使用0.0.0.0
### ch5.3.6 层次化配置
我们从configuration.yaml文件读取配置。
采用更精细的配置方法：
1. 基础配置文件，也就是共享的部分。
2. 特定于环境的配置文件，如主机
3. 环境变量APP_ENVIRONMENT用于确定运行的环境
所有的配置文件都位于一个根目录下: configuration
### ch5.3.7 数据库链接
30s是sqlx中从连接池中获取链接的默认超时时间。
我们可以设置一个更短的超时时间.
使用--network=host运行应用程序容器，就像使用Postgres一样
使用docker-compose
### ch5.3.8 优化Docker镜像
#### ch5.3.8.1 Docker镜像大小
我们不会在托管应用程序的机器上运行Docker build，而是通过docker pull来下载Docker镜像。
docker images zero2prod可以查看镜像大小
.dockerignore可以用来决定哪些文件应该被忽略
COPY指令不会将其构建到镜像中。
Docker的多阶段构建
1. builder阶段：生成一个已编译的二进制文件
2. runtime阶段，运行二进制文件

runtime是最后的镜像。builder阶段不影响镜像的大小。builder阶段是一个中间步骤，其产物是一个已编译的二进制文件。
我们可以更进一步，在runtime阶段使用rust:1.97.1-slim.
我们还可以通过缩减整条Rust工具链的大小再次缩小镜像的体积。
我们可以使用纯净的操作系统作为runtime阶段的基础镜像(debian:bullseye-slim)。
#### ch5.3.8.2 缓存Rust的Docker构建
Rust的编译时间过长，尤其是优化构建--release构建方式。
我们可以利用Docker的分层缓存功能来解决每次构建的时间过长的问题。
Dockerfile中每一个命令都会创建一个分层：之前的状态和当前的执行指定命令之后状态的差异。
分层被缓存起来：如果一个操作的起始点没有改变，而且命令本身也没有改变，Docker就不会产生额外的计算，而是直接从本地缓存中获取结果。
优化策略就是调整操作顺序：
经常改变的文件放在最后。
复制锁定文件，建立依赖关系，复制源代码，然后构建项目。
## ch5.4 部署到DigitalOcean平台
### ch5.4.1 安装
注册使用DigitalOcean
### ch5.4.2 应用规范
我们需要提供一个声明性的配置文件，让我们指定应用部署应该是什么样子的。 App spec也就是。
DigitalOcean通过提供一个证书，设置了HTTPS，并将HTTPS流量重定向到应用规范中指定的端口。
POST /subscriptions端点失败，在生产环境中没有提供数据库。
### ch5.4.3 如何使用环境变量注入加密信息
我们使用环境变量来定制Settings结构体中的任何值，从而取代配置文件中指定的内容。
环境变量对config包视为字符串，如果使用serde的反序列化，无法提取整数。
我们可以指定一个自定义的反序列化函数。
### ch5.4.5 连接到DigitalOcean的Postgres实例
在生产环境中为客户端/数据库提供传输层加密是有必要的，就是去支持SSL模式。
# ch6 拒绝无效的订阅者（第一部分）
POST /subscriptions端点只实现了一个最基本的通过Form提取器反序列化出FormData，但是没有验证提取出来的数据的正确性。
## ch6.1 需求
### ch6.1.1 姓名约束
可以简单的要求姓名字段非空。
### ch6.1.2 安全约束
表单和用户输入作为主要攻击目标(SQL注入，服务器上运行恶意代码，搞崩服务等)
可能遇到的问题：
1. 拒绝服务
2. 窃取数据
3. 网络钓鱼

采用分层安全方法来解决这些威胁：通过在技术栈的多个层面上采取应对策略来降低这些威胁的风险。
1. 限制最大长度
2. 拒绝包含问题字符的名字。/()"<>\{}
## ch6.2 第一次实现
一个扩展trait，为'String'和'&str'提供‘graphemes'方法
unicode_segmentation::UnicodeSegmentation
## ch6.3 漏洞百出的验证
insert_subscriber方法要保证form.name不为空。
虽然我们使用了is_valid_name进行验证，但是是在当前函数外部进行的。
在大型项目中，检查函数的所有调用点以确保事先执行了某一个验证步骤是不切实际的。
如果一定要用is_valid_name, 我们唯一可行的方案是在所有要求form.name非空的函数内部调用。
但是如果insert_subscriber会被拆分若干个子函数，每一个函数又要调用is_valid_name.
扩展性极差。
由于is_valid_name是一个验证函数：在程序执行流程的某一个特定点，一组条件得到了验证，但是关于输入数据中附加结构的信息没有被存储在任何地方。会立即消失，导致无法重用。
我们需要添加一个解析函数--接受非结构化输入的程序。如果一组条件成立，返回更加结构化的输出。
## ch6.4 类型驱动开发
我们创建了一个元组结构体SubscriberName。
注意，其字段是私有的，所以其元组结构体构造器也是私有的。无法直接访问。
parse方法是构建SubscriberName的唯一方法，任何SubscriberName实例都能满足约束。
我们从所有的订阅者的姓名必须符合一些约束条件开始，确定了一个潜在的问题（在调用insert_subscriber之前可能忘记了验证输入），并利用Rust的类型系统完成该功能。
我们通过构造类型使得错误的使用方式无法被编译出来。
这种技术叫做”类型驱动开发“。
类型驱动开发：将我们试图建模的domain约束条件编码到类型系统中，并依靠编译器来确保这些约束得到执行。
（我们的例子：String不为空的约束 -> SubscriberName）

## ch6.5 所有权遇到不变量
为什么不能将SubscriberName(String) -> SubscriberName(pub String)呢？
1. 其他开发人员可以直接访问字段，构造一个不满足SubscriberName的约束条件的值
2. 即使使用了，也可以通过修改导致其不在满足约束条件。

可以利用Rust所有权机制解决。（如何访问私有字段的值）
我们可以通过公有方法访问，可以获取值所有权，原来的SubscriberName失效，成员被移动了。可以获取值的共享引用，也就是不可变引用。
### ch6.5.1 AsRef
AsRef可以用来获得一个结构体或者元组的私有字段的不可变引用。
````
pub trait AsRef<T: ?Sized> {
 /// 执行转换
 fn as_ref(&self) -> &T;
}
````
AsRef是一个引用转换trait。
如果类型Self与T足够相似，就可以实现该trait，通过&self,得到&T.
## ch6.6 panic
更优雅地处理错误。
panic被用来处理不可恢复的错误！显然这里不适合。
## ch6.7 Result --将错误作为值
### ch6.7.1 使用解析函数返回Result类型
重构SubscriberName::parse方法，输入无效返回Result<SubscriberName, Box<dyn Error>>
## ch6.8 精确的断言错误：claim
Rust提供的assert!宏进行断言时，产生的错误很难糟糕。
没有错误的详细消息。我们可以使用claim crate获取更多消息。
## ch6.9 单元测试
test模块
## ch6.10 处理Result
使得验证错误时返回400 Bad Request
### ch6.10.1 match
REST API是一种设计风格 URL定位资源，POST是HTTP请求方法，输入是表单数据，输出是HTTP状态码。
### ch6.10.2 ?操作符
是抛出错误的语法糖，只能用于Result枚举和Option
### ch6.10.3 400的请求错误
## ch6.11 电子邮件地址格式
如何确定有效的电子邮件地址。
Rust生态中有一个 validator包用来验证邮箱的有效性。
## ch6.12 SubscriberEmail类型
### ch6.12.1 拆分domain子模块
### ch6.12.2 新类型的框架
## ch6.13 属性测试
不是验证一组特定的输入是否被正确解析，而是构建一个随机生成器来产生有效的值。并检查解析器是否会拒绝它们。
也就是验证实现是否显示了一个特定的属性。不会拒绝任何有效的电子邮件地址。
### ch6.13.1 使用fake生成随机测试数据
fake提供了基本数据类型和高级对象的生成逻辑。
生成一个随机数据进行测试可能需要手动运行多次测试进行调整验证才能触碰到边界。
一个简单高效的方法是添加for循环。
### ch6.13.2 quickcheck和proptest
这是两个主流的属性测试选项
### ch6.13.3 quickcheck入门
quickcheck会以配置好的迭代次数，默认是100循环调用函数，每次迭代中都会随机生成满足参数列表条件的参数，传入，进行验证，判断函数是否返回true
如果是false，quickcheck会不断缩小迭代次数获得失败用例。
### ch6.13.4 Arbitrary trait
quickcheck通过Arbitrary trait生成随机输入数据。
````
pub trait Arbitrary: Clone + Send + 'static {
   fn arbitrary<G: Gen>(g: &mut G) -> Self;
   
   fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
       empty_shrinker()
   }
}
````
arbitrary 给定一个随机源，返回一个该实例的类型。
shrink 返回该类型的一个逐渐变小的实例序列。
## ch6.14 请求体验证
### ch6.14.1 使用TryFrom重构
TryFrom： 两种类型之间可能失败的转换，同时会获取输入值的所有权
````
pub trait TryFrom<T>: Sized {
 /// 在转换错误的情况返回错误类型
 type Error;
 
 /// 执行转换
 fn try_from(value: T) -> Result<Self, Self::Error>;
}
````
我们实现了TryFrom调用的是try_into()
````
pub trait TryInto<T> {
 type Error;
 fn try_into(self) -> Result<T, Self::Error>;
}
````

