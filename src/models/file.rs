use ::crypto::{self, Key};
use ::jedi::Value;
use ::error::{TResult, TError};
use ::storage::Storage;
use ::models::model::Model;
use ::models::protected::{Keyfinder, Protected};
use ::models::note::Note;
use ::models::sync_record::{SyncAction, SyncType, SyncRecord};
use ::models::validate::Validate;
use ::sync::sync_model::{self, SyncModel, MemorySaver};
use ::turtl::Turtl;
use ::std::mem;
use ::util;
use ::std::fs;
use ::std::io::prelude::*;
use ::std::path::PathBuf;
use ::glob;

/// Return the location where we store files
pub fn file_folder() -> TResult<String> {
    util::file_folder(Some("files"))
}

protected! {
    /// Defines the object we find inside of Note.File (a description of the
    /// note's file with no actual file data...name, mime type, etc).
    #[derive(Serialize, Deserialize)]
    pub struct File {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[protected_field(public)]
        pub size: Option<u64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[protected_field(private)]
        pub name: Option<String>,
        #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
        #[protected_field(private)]
        pub ty: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[protected_field(private)]
        pub meta: Option<Value>,
    }
}

protected! {
    /// Defines the object that holds actual file body data separately from the
    /// metadata that lives in the Note object.
    #[derive(Serialize, Deserialize)]
    #[protected_modeltype(file)]
    pub struct FileData {
        #[serde(with = "::util::ser::base64_converter")]
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        #[protected_field(private)]
        pub data: Option<Vec<u8>>,
    }
}

make_storable!(FileData, "files");
impl Validate for FileData {}

impl SyncModel for FileData {
    // this one is weird. we detect if this is saving from an incoming sync
    // (API -> turtl), and if so, save a SyncRecord to the `sync` table w/ sync
    // type FileIncoming (lets the incoming file sync system know we have a
    // customer), OR if it's an outgoing sync, DO NOTHING.
    fn db_save(&self, db: &mut Storage, sync_item: Option<&SyncRecord>) -> TResult<()> {
        // only incoming syncs have a non-None value for sync_item. we will use
        // this to detect if is incoming vs outgoing.
        if let Some(sync) = sync_item {
            // ha ha! incoming..
            let mut sync_record = sync.clone_shallow();
            sync_record.generate_id()?;
            // change the type. heh heh, yes, very clever indeed...
            sync_record.ty = SyncType::FileIncoming;
            // ...and queue the file for download in our incoming sync queue
            sync_record.db_save(db, None)?;
        }
        Ok(())
    }

    // remove the file
    fn db_delete(&self, _db: &mut Storage, _sync_item: Option<&SyncRecord>) -> TResult<()> {
        let id = self.id_or_else()?;

        // we could use FileData::file_finder here, but we actually do want to
        // find ALL files with this note ID and remove them. just a paranoid
        // precaution.
        let mut filepath = PathBuf::from(file_folder()?);
        filepath.push(FileData::filebuilder(None, Some(&id)));
        let pathstr = match filepath.to_str() {
            Some(x) => x,
            None => return TErr!(TError::BadValue(format!("invalid path: {:?}", filepath))),
        };
        let files = glob::glob(&pathstr)?;
        for file in files {
            fs::remove_file(&file?)?;
        }
        Ok(())
    }

    // override the sync model's outgoing default fn. we need to set our sync
    // type by hand.
    fn outgoing(&self, action: SyncAction, user_id: &String, db: &mut Storage, skip_remote_sync: bool) -> TResult<()> {
        let ty = match action {
            SyncAction::Delete => {
                self.db_delete(db, None)?;
                SyncType::File
            }
            _ => {
                self.db_save(db, None)?;
                SyncType::FileOutgoing
            }
        };
        if skip_remote_sync { return Ok(()); }

        let mut sync_record = SyncRecord::default();
        sync_record.generate_id()?;
        sync_record.action = action;
        sync_record.user_id = user_id.clone();
        sync_record.ty = ty;
        sync_record.item_id = self.id_or_else()?;

        match sync_record.action {
            SyncAction::Delete => {
                sync_record.data = Some(json!({
                    "id": self.id().expect("turtl::FileData.outgoing() -- delete -- self.id() is None").clone(),
                }));
            }
            _ => {
                sync_record.data = Some(self.data_for_storage()?);
            }
        }
        sync_record.db_save(db, None)
    }
}

impl Keyfinder for FileData {}
impl MemorySaver for FileData {
    fn mem_update(self, turtl: &Turtl, sync_item: &mut SyncRecord) -> TResult<()> {
        let action = sync_item.action.clone();
        match action {
            SyncAction::Delete => {
                // unwrap is ok. we will always have an id. hopefully. no, but
                // we will.
                let note_id = self.id().expect("turtl::FileData::.mem_update() -- delete -- self.id() IS NONE AAARRRGGGGHGHHGHHH").clone();
                let mut notes = turtl.load_notes(&vec![note_id.clone()])?;
                if notes.len() == 0 { return Ok(()); }
                let note = &mut notes[0];
                note.has_file = false;
                note.file = None;
                sync_model::save_model(SyncAction::Edit, turtl, note, true)?;
            }
            _ => {}
        }
        Ok(())
    }
}

impl FileData {
    /// Builds a standard filename
    fn filebuilder(user_id: Option<&String>, note_id: Option<&String>) -> String {
        // wildcard, bitches. YEEEEEEEEHAWW!!!
        let wildcard = String::from("*");
        format!(
            "u_{}.n_{}.enc",
            user_id.unwrap_or(&wildcard),
            note_id.unwrap_or(&wildcard),
        )
    }

    /// Find the PathBuf for a file, given the pieces that build the filename
    pub fn file_finder_all(user_id: Option<&String>, note_id: Option<&String>) -> TResult<Vec<PathBuf>> {
        let mut filepath = PathBuf::from(file_folder()?);
        filepath.push(FileData::filebuilder(user_id, note_id));
        let pathstr = match filepath.to_str() {
            Some(x) => x,
            None => return TErr!(TError::BadValue(format!("invalid path: {:?}", filepath))),
        };
        let files = glob::glob(pathstr)?;
        let mut res = Vec::new();
        for file in files {
            res.push(file?);
        }
        Ok(res)
    }

    /// Find the PathBuf for a file, given the pieces that build the filename
    pub fn file_finder(user_id: Option<&String>, note_id: Option<&String>) -> TResult<PathBuf> {
        let mut files = FileData::file_finder_all(user_id, note_id)?;
        if files.len() < 1 {
            return TErr!(TError::NotFound(format!("file not found")));
        }
        Ok(files.swap_remove(0))
    }

    /// Given a user_id/note_id, return the PathBuf to a location the file
    /// should be saved.
    pub fn new_file(user_id: &String, note_id: &String) -> TResult<PathBuf> {
        let mut filepath = PathBuf::from(file_folder()?);
        filepath.push(FileData::filebuilder(Some(user_id), Some(note_id)));
        Ok(filepath)
    }

    /// Load a note's file, if we have one.
    pub fn load_file(turtl: &Turtl, note: &Note) -> TResult<Vec<u8>> {
        let note_id = note.id_or_else()?;
        // get the note's space id
        let space_id = Note::get_space_id(turtl, &note_id);
        let note_key = Key::random().unwrap();

        let profile_guard = lockr!(turtl.profile);
        // iterate through the spaces in this profile to find the space that contains this note
        for space in profile_guard.spaces {
            if space.id().unwrap().to_string() == space_id.unwrap() {
                note_key = Key::new(crypto::from_base64(&space.vdb.unwrap().query(note_id)).unwrap());
            }
            break;
        }

        drop(profile_guard);
        // let note_key = note.key_or_else()?;

        let filename = FileData::file_finder(None, Some(&note_id))?;
        let enc = {
            let mut file = fs::File::open(filename)?;
            let mut enc = Vec::new();
            file.read_to_end(&mut enc)?;
            enc
        };

        // decrypt the file using the turtl standard serialization format
        let data = turtl.work.run(move || {
            crypto::decrypt(&note_key, enc)
                .map_err(|e| From::from(e))
        })?;

        Ok(data)
    }

    /// Encrypt/save this file
    pub fn save(&mut self, turtl: &Turtl, note: &mut Note) -> TResult<()> {
        // grab some items we'll need to do our work (user_id/note_id for the
        // filename, note_key for encrypting the file).
        let user_id = turtl.user_id()?;
        let note_id = note.id_or_else()?;
        // get the note's space id
        let space_id = Note::get_space_id(turtl, &note_id);
        let note_key = Key::random().unwrap();

        let profile_guard = lockr!(turtl.profile);
        // iterate through the spaces in this profile to find the space that contains this note
        for space in profile_guard.spaces {
            if space.id().unwrap().to_string() == space_id.unwrap() {
                note_key = Key::new(crypto::from_base64(&space.vdb.unwrap().query(note_id)).unwrap());
            }
            break;
        }

        drop(profile_guard);
        // let note_key = note.key_or_else()?;

        // the file id should ref the note
        self.id = Some(note_id.clone());

        // rip the `data` field out of the FileData object
        let mut data: Option<Vec<u8>> = None;
        mem::swap(&mut data, &mut self.data);

        // unwrap our data
        let data = match data {
            Some(x) => x,
            None => return TErr!(TError::MissingField(format!("FileData.data"))),
        };

        // encrypt the file using the turtl standard serialization format
        let enc = turtl.work.run(move || {
            crypto::encrypt(&note_key, data, crypto::CryptoOp::new("chacha20poly1305")?)
                .map_err(|e| From::from(e))
        })?;

        // now, save the encrypted file data to disk
        let mut filepath = PathBuf::from(file_folder()?);
        util::create_dir(&filepath)?;
        filepath.push(FileData::filebuilder(Some(&user_id), Some(&note_id)));
        let mut fs_file = fs::File::create(&filepath)?;
        fs_file.write_all(enc.as_slice())?;

        // phew, now that all went smoothly, create a sync record for the saved
        // file (which will let the sync system know to upload our heroic file)
        let create_sync = move || -> TResult<()> {
            let mut db_guard = lock!(turtl.db);
            let db = match db_guard.as_mut() {
                Some(x) => x,
                None => return TErr!(TError::MissingField(format!("Turtl.db"))),
            };

            // run the sync. this would normally write an object to the "files"
            // table, but since we've overwritten db_save() to do NOTHING we can
            // rest easy here knowing we won't get random records in tables that
            // shouldn't exist.
            self.outgoing(SyncAction::Add, &user_id, db, false)?;
            Ok(())
        };
        match create_sync() {
            Ok(_) => (),
            Err(e) => {
                match fs::remove_file(&filepath) {
                    Ok(_) => {},
                    Err(e) => {
                        error!("FileData.save() -- error removing saved file: {}", e);
                    }
                }
                return Err(e);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::jedi;

    #[test]
    fn filedata_serializes_to_from_base64() {
        let filedata: Vec<u8> = vec![73, 32, 67, 65, 78, 39, 84, 32, 66, 69, 76, 73, 69, 86, 69, 32, 73, 84, 39, 83, 32, 78, 79, 84, 32, 71, 79, 78, 79, 82, 82, 72, 69, 65, 33, 33];
        let mut file: FileData = Default::default();
        file.data = Some(filedata.clone());

        let ser = jedi::stringify(&file).unwrap();
        assert_eq!(ser, r#"{"body":null,"data":"SSBDQU4nVCBCRUxJRVZFIElUJ1MgTk9UIEdPTk9SUkhFQSEh"}"#);

        let file2: FileData = jedi::parse(&ser).unwrap();
        assert_eq!(file2.data.as_ref().unwrap(), &filedata);
    }

    #[test]
    fn can_save_and_load_files() {
        let turtl = ::turtl::tests::with_test(true);
        let user_id = turtl.user_id().unwrap();

        let mut note: Note = jedi::from_val(json!({
            "space_id": "6969",
            "user_id": user_id.clone(),
        })).unwrap();
        note.generate_id().unwrap();
        note.generate_key().unwrap();

        let filedata = jedi::stringify(&json!({
            "name": "flippy",
            "likes": "slippy",
            "dislikes": "slappy",
            "age": 42,
            "lives": {
                "city": "santa cruz brahhhh"
            }
        })).unwrap();

        let mut file: FileData = Default::default();
        file.data = Some(Vec::from(filedata.as_bytes()));

        // talked to drew about encrypting and saving the file. sounds good.
        file.save(&turtl, &mut note).unwrap();
        let loaded = FileData::load_file(&turtl, &note).unwrap();

        // see if the file contents match after decryption
        assert_eq!(String::from_utf8(loaded).unwrap(), r#"{"age":42,"dislikes":"slappy","likes":"slippy","lives":{"city":"santa cruz brahhhh"},"name":"flippy"}"#);

        let mut db_guard = lock!(turtl.db);
        let db = db_guard.as_mut().unwrap();
        file.db_delete(db, None).unwrap();

        match FileData::load_file(&turtl, &note) {
            Ok(_) => panic!("Found file for note {}, should be deleted", note.id().as_ref().unwrap()),
            Err(e) => {
                let e = e.shed();
                match e {
                    // amazing, heh heh.
                    TError::NotFound(_) => {},
                    _ => panic!("{}", e),
                }
            },
        }
    }
}
