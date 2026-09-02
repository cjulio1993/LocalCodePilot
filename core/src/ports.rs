#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Port(u16);

impl Port {
    pub fn new(value: u16) -> Option<Self> {
        (value > 0).then_some(Self(value))
    }
    pub fn value(self) -> u16 {
        self.0
    }
}
