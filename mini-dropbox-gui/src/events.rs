pub enum AppEvent {
    ReadyToPublishUser,
    AllSystemsGo,
    RegisterSuccess,

    UpdateActiveUsersList(Vec<String>),

    FatalError(anyhow::Error),
}
