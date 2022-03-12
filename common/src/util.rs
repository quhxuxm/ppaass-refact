use uuid::Uuid;

pub fn generate_uuid() -> String {
    let uuid_str = Uuid::new_v4().to_string();
    uuid_str.replace('-', "")
}
