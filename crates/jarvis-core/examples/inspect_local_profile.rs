// Local credential diagnostic and explicit repair. Never prints secrets.
use sha2::{Digest,Sha256};
fn main(){
    let root=std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("com.diberlog.jarvis.syncv2lab");
    let canonical=root.canonicalize().unwrap();
    let expected=Sha256::digest(canonical.to_string_lossy().as_bytes()).iter().map(|b|format!("{b:02x}")).collect::<String>();
    println!("expected_account={expected}");
    let raw:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("sync-v2-group.json")).unwrap()).unwrap();
    let arguments=std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().is_some_and(|arg|arg=="--repair"){
        let source=arguments.get(1).expect("Specify a verified existing JARVIS credential account");
        assert_ne!(source,&expected);
        let mut bytes=keyring::Entry::new("JARVIS_ALTRON.local-data.v1",source).unwrap().get_secret().unwrap();
        let key=<&[u8;32]>::try_from(bytes.as_slice()).expect("Invalid key length");
        jarvis_core::secure_storage::open_with_key(&raw,"sync-v2-group",key).expect("Source key does not authenticate this profile");
        for file in ["devices.json","tls.json"]{
            let raw:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("lan-sync").join(file)).unwrap()).unwrap();
            jarvis_core::secure_storage::open_with_key(&raw,"lan-sync-local",key).expect("Mixed profile keys; nothing changed");
        }
        let target=keyring::Entry::new("JARVIS_ALTRON.local-data.v1",&expected).unwrap();
        let mut previous=target.get_secret().expect("Existing target entry required");
        let backup=format!("{expected}.before-repair-{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
        let saved=keyring::Entry::new("JARVIS_ALTRON.local-data.repair-backup",&backup).unwrap();
        saved.set_secret(&previous).unwrap();assert!(saved.get_secret().unwrap()==previous,"OS backup did not verify");previous.fill(0);
        target.set_secret(&bytes).unwrap();assert!(target.get_secret().unwrap()==bytes,"OS credential did not verify");bytes.fill(0);
        println!("Repaired OS credential binding; encrypted profile files unchanged; previous key retained in OS credential storage.");
        return;
    }
    for account in arguments{
        let result=keyring::Entry::new("JARVIS_ALTRON.local-data.v1",&account).and_then(|e|e.get_secret());
        match result{Ok(mut bytes)=>{let valid=if let Ok(key)=<&[u8;32]>::try_from(bytes.as_slice()){jarvis_core::secure_storage::open_with_key(&raw,"sync-v2-group",key).is_ok()}else{false};bytes.fill(0);println!("account={account} authenticates_group={valid}");},Err(_)=>println!("account={account} unavailable")}
    }
}
