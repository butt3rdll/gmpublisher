use std::{collections::{HashMap, HashSet}, fs::File, hash::Hash, io::{Read, BufReader, BufWriter}, path::PathBuf};

lazy_static! {
	static ref RE_BUNDLE_DATA: Regex = regex::RegexBuilder::new(r#"^[ \t]*(?:(?:("|'|\[(=*)\[)(\d+)(?:\1|\]\2\]))|--#[ \t]*+(.+?)(?:[ \t]+(.+)|$))"#).multi_line(true).build().unwrap();
}

use chrono::Utc;
use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use steamworks::PublishedFileId;

enum BundleError {
	ParseError,
	NoItemsFound,
}

#[derive(Serialize, Deserialize)]
struct BundleItem {
	id: PublishedFileId,
	added: chrono::DateTime<Utc>,
}
impl PartialEq for BundleItem {
	fn eq(&self, other: &Self) -> bool {
		self.id == other.id
	}
}
impl Eq for BundleItem {}
impl PartialOrd for BundleItem {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		self.added.partial_cmp(&other.added)
	}
}
impl Ord for BundleItem {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.added.cmp(&other.added)
	}
}

#[derive(Serialize, Deserialize)]
struct BundleCollectionLink {
	id: PublishedFileId,
	include: Vec<PublishedFileId>,
	exclude: Vec<PublishedFileId>
}

#[derive(Serialize, Deserialize)]
struct Bundle {
	id: u16,
	name: String,
	updated: chrono::DateTime<Utc>,
	collection: Option<BundleCollectionLink>,
	items: Vec<PublishedFileId>,
}
impl PartialEq for Bundle {
	fn eq(&self, other: &Self) -> bool {
		self.id == other.id
	}
}
impl Eq for Bundle {}
impl PartialOrd for Bundle {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		self.updated.partial_cmp(&other.updated)
	}
}
impl Ord for Bundle {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.updated.cmp(&other.updated)
	}
}
impl Bundle {
	pub fn import(src: String) -> Result<Bundle, BundleError> {
		let mut bundle_start = false;

		let mut name = String::new();
		let mut collection: Option<BundleCollectionLink> = None;
		let mut updated = chrono::Utc::now();
		let mut items = Vec::with_capacity(4096);

		for data in RE_BUNDLE_DATA.captures_iter(&src) {
			if let Some(key) = data.get(4) {
				if key.as_str() == "bundle" {
					if !bundle_start {
						bundle_start = true;
					} else {
						break;
					}
				} else {
					let val = match data.get(5) {
						Some(val) => val,
						None => continue
					};
					match key.as_str() {
						"name" => name = val.as_str().to_string(),
						"collection" => if let Ok(id) = val.as_str().parse::<u64>() {
							collection = Some(BundleCollectionLink {
								id: PublishedFileId(id),
								include: Vec::with_capacity(4096),
								exclude: Vec::new(),
							});
						},
						"updated" => if let Ok(parsed) = chrono::DateTime::parse_from_rfc2822(val.as_str()) {
							updated = parsed.with_timezone(&Utc);
						},
						_ => {}
					}
				}
			} else if let Some(item) = data.get(3) {
				items.push(PublishedFileId(match item.as_str().parse::<u64>() {
					Ok(id) => id,
					Err(_) => continue
				}));
			} else {
				#[cfg(debug_assertions)]
				panic!("Unexpected match when parsing bundle data");
			}
		}

		if items.is_empty() {
			return Err(BundleError::NoItemsFound);
		}

		if let Some(ref mut collection) = collection {
			if let Some(collection_items) = steam!().fetch_collection_items(collection.id) {
				for item in collection_items {
					match items.binary_search(&item) {
						Ok(pos) => {
							items.remove(pos);
							collection.include.push(item);
						},
						Err(_) => {
							collection.exclude.push(item);
						}
					}
				}
				collection.include.shrink_to_fit();
				collection.exclude.shrink_to_fit();
			}
		}

		let id = BUNDLES.lock().id + 1; // TODO potential deadlock?
		Ok(Bundle {
		    id,
		    name,
		    updated,
		    collection,
		    items,
		})
	}

	pub fn export(&self, item_names: HashMap<PublishedFileId, String>, collection_name: Option<&str>) -> String {
		// TODO convert these to write!(export, ...)

		let mut export = String::with_capacity(1000000);
		export.push_str("-- generated by gmpublisher\n");
		export.push_str("-- https://gmpublisher.download\n");
		export.push_str("--# bundle\n");

		export.push_str("--# name ");
		export.push_str(&self.name);
		export.push('\n');

		if let Some(ref collection) = self.collection {
			export.push_str("--# collection ");
			export.push_str(&collection.id.0.to_string());
			export.push('\n');
		}

		export.push_str("--# updated ");
		export.push_str(&self.updated.to_rfc2822());
		export.push('\n');

		export.push_str("for _,w in ipairs({\n\n");

		for item in self.items.iter() {
			export.push('\"');
			export.push_str(&item.0.to_string());
			if let Some(name) = item_names.get(item) {
				export.push_str("\" -- ");
				export.push_str(name);
				export.push('\n');
			}
		}

		if let Some(ref collection) = self.collection {
			export.push_str("\n-- Collection\n");
			if let Some(collection_name) = collection_name {
				export.push_str("-- ");
				export.push_str(collection_name);
				export.push('\n');
			}
			export.push_str("-- https://steamcommunity.com/sharedfiles/filedetails/?id=");
			export.push_str(&collection.id.0.to_string());
			export.push('\n');
			for item in collection.include.iter() {
				export.push('\"');
				export.push_str(&item.0.to_string());
				if let Some(name) = item_names.get(item) {
					export.push_str("\" -- ");
					export.push_str(name);
					export.push('\n');
				}
			}
		}

		export.push_str("\n}) do resource.AddWorkshop(w) end");

		export.shrink_to_fit();
		export
	}
}

#[derive(Serialize, Deserialize)]
pub struct Bundles {
	saved: Vec<Bundle>,
	id: u16,
}
impl Bundles {
	pub fn init() -> Self {
		let mut saved = Vec::new();
		let mut id = 0;

		std::fs::create_dir_all(&*bundles_path()).expect("Failed to create content generator bundles directory");

		if let Ok(dir) = bundles_path().read_dir() {
			for entry in dir {
				ignore! { try_block!({
					let entry = entry?;
					let contents: Bundle = bincode::deserialize_from(BufReader::new(File::open(entry.path())?))?;
					id = id.max(contents.id);

					saved.insert(
						match saved.binary_search(&contents) {
							Ok(pos) => pos,
							Err(pos) => pos,
						},
						contents,
					);
				}) };
			}
		}

		Self { saved, id }
	}
}

lazy_static! {
	pub static ref BUNDLES: Mutex<Bundles> = Mutex::new(Bundles::init());
}

fn bundles_path() -> PathBuf {
	app_data!().user_data_dir().join("content_generator")
}

#[tauri::command]
fn get_bundles() -> &'static Vec<Bundle> {
	unsafe { &*(&BUNDLES.lock().saved as *const _) }
}

#[tauri::command]
fn update_bundle(bundle: Bundle) -> bool {
	try_block!({
		let mut content_generator = BUNDLES.lock();

		let f = File::create(bundles_path().join(bundle.id.to_string()))?;
		bincode::serialize_into(BufWriter::new(f), &bundle)?;

		match content_generator.saved.binary_search(&bundle) {
			Ok(pos) => content_generator.saved[pos] = bundle,
			Err(pos) => content_generator.saved.insert(pos, bundle),
		}
	})
	.is_ok()
}