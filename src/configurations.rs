//! src/configurations.rs
#[derive(serde::Deserialize)]
pub struct Settings {
    pub database: DatabaseSettings, // 数据库链接配置
    pub application_port: u16, // 应用端口
}

#[derive(serde::Deserialize)]
pub struct DatabaseSettings {
    pub username: String,
    pub password: String,
    pub port: u16,
    pub host: String,
    pub database_name: String,
}

// 读取配置信息
pub fn get_configurations() -> Result<Settings, config::ConfigError> {
    // 初始化配置读取器
    let settings = config::Config::builder()
        // 从一个叫做'configurations.yaml'的文件中读取配置值
        .add_source(config::File::new("configuration.yaml", config::FileFormat::Yaml))
        .build()?;
    // 尝试将读取的类型转化为Settings类型
    settings.try_deserialize::<Settings>()
}

// 使用PgConnection::connect进行数据库链接， 提供一个URL
impl DatabaseSettings {
    pub fn connection_string(&self) -> String {
        format!(
            "postgres://{}:{}@{}:{}/{}",
            self.username, self.password, self.host, self.port, self.database_name
        )
    }
    
    pub fn connection_string_without_db(&self) -> String {
        format!("postgres://{}:{}@{}:{}", self.username, self.password, self.host, self.port)
    }
}