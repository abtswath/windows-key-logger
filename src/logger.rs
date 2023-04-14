use log::{set_boxed_logger, set_max_level, Level, LevelFilter, Log, Metadata};

pub struct Logger {
    pub level: Level,
}

impl Logger {
    pub fn new(level: Level) -> Logger {
        Logger { level }
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            println!("{} - {}", record.level(), record.args())
        }
    }

    fn flush(&self) {}
}

pub fn init(level: Level) -> Result<(), log::SetLoggerError> {
    set_boxed_logger(Box::new(Logger::new(level))).map(|()| set_max_level(LevelFilter::Debug))
}
