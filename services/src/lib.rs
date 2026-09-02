use localcodepilot_core::ports::Port;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    Mysql,
    Postgresql,
    Redis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub kind: ServiceKind,
    pub port: Port,
}

impl ServiceKind {
    pub fn default_port(self) -> Port {
        Port::new(match self {
            Self::Mysql => 3306,
            Self::Postgresql => 5432,
            Self::Redis => 6379,
        })
        .expect("service ports are valid")
    }
}
