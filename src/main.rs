#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate lazy_static;
use serde::{Deserialize, Serialize};

#[macro_use]
extern crate rocket;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::str;
use std::sync::Mutex;
use std::time::Instant;

use std::fs::File;
use std::io::Read;
use std::result::Result;

use libxml::xpath::Context;
use llamapun::data::{Corpus, Document};
use llamapun::util::data_helpers;
use regex::Regex;

// use tensorflow::Code;
use tensorflow::ImportGraphDefOptions;
use tensorflow::{Graph, Session, SessionOptions, SessionRunArgs, Tensor};

use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Header;
use rocket::response::content;
use rocket::response::status::NotFound;
use rocket::response::NamedFile;
use rocket::State;
use rocket_contrib::json::Json;
use rocket_contrib::templates::Template;

// We need a singleton Corpus object to hold the various expensive state objects
static MAX_WORD_LENGTH: usize = 25;
static PARAGRAPH_SIZE: usize = 480;

lazy_static! {
  static ref IS_NUMERIC: Regex =
    Regex::new(r"^-?(?:\d+)(?:[a-k]|(?:\.\d+(?:[eE][+-]?\d+)?))?$").unwrap();
  static ref DICTIONARY: Mutex<HashMap<String, u64>> = {
    let json_file = File::open(Path::new("word_index.json")).expect("file not found");
    let dictionary: HashMap<String, u64> =
      serde_json::from_reader(json_file).expect("error while reading json");
    Mutex::new(dictionary)
  };
  static ref TF_GRAPH: Graph = {
    let filename = "13_class_statement_classification_bilstm.pb";
    println!("-- loading TF model {}", filename);
    let mut graph = Graph::new();
    let mut proto = Vec::new();
    File::open(filename)
      .unwrap()
      .read_to_end(&mut proto)
      .unwrap();
    println!("-- reading in graph data");
    graph
      .import_graph_def(&proto, &ImportGraphDefOptions::new())
      .unwrap();
    println!("-- graph imported");
    graph
  };
}

#[derive(Serialize)]
struct TemplateContext {
  global: HashMap<String, String>,
}
impl Default for TemplateContext {
  fn default() -> Self {
    TemplateContext {
      global: HashMap::new(),
    }
  }
}

pub struct CORS();

impl Fairing for CORS {
  fn info(&self) -> Info {
    Info {
      name: "Add CORS headers to requests",
      kind: Kind::Response,
    }
  }

  fn on_response(&self, request: &rocket::Request, response: &mut rocket::Response) {
    if request.method() == rocket::http::Method::Options
      || response.content_type() == Some(rocket::http::ContentType::JSON)
    {
      response.set_header(Header::new("Access-Control-Allow-Origin", "*"));
      response.set_header(Header::new(
        "Access-Control-Allow-Methods",
        "POST, GET, OPTIONS",
      ));
      response.set_header(Header::new("Access-Control-Allow-Headers", "Content-Type"));
      response.set_header(Header::new("Access-Control-Allow-Credentials", "true"));
      response.set_header(Header::new(
        "Content-Security-Policy-Report-Only",
        "default-src https:; report-uri /csp-violation-report-endpoint/",
      ));
    }

    if request.method() == rocket::http::Method::Options {
      response.set_header(rocket::http::ContentType::Plain);
      response.set_sized_body(Cursor::new(""));
    }
  }
}

#[get("/favicon.ico")]
fn favicon() -> Result<NamedFile, NotFound<String>> {
  let path = Path::new("public/").join("favicon.ico");
  NamedFile::open(&path).map_err(|_| NotFound(format!("Bad path: {:?}", path)))
}

#[get("/public/<file..>")]
fn files(file: PathBuf) -> Result<NamedFile, NotFound<String>> {
  let path = Path::new("public/").join(file);
  NamedFile::open(&path).map_err(|_| NotFound(format!("Bad path: {:?}", path)))
}

#[get("/")]
fn root() -> Template {
  let mut context = TemplateContext::default();
  let mut global = HashMap::new();
  global.insert(
    "title".to_string(),
    "A Demo for Scientific Paragraph Classification".to_string(),
  );
  global.insert(
    "description".to_string(),
    "Interactive editing and automatic classification of scientific paragraphs, via latexml and llamapun".to_string(),
  );

  context.global = global;

  Template::render("overview", context)
}

#[derive(Serialize, Deserialize, Debug)]
struct LatexmlRequest {
  tex: String,
  preamble: String,
  comments: String,
  post: String,
  timeout: String,
  format: String,
  whatsin: String,
  whatsout: String,
  pmml: String,
  cmml: String,
  mathtex: String,
  mathlex: String,
  nodefaultresources: String,
  preload: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct LatexmlResponse {
  result: String,
  status: String,
  status_code: u8,
  log: String,
}

impl LatexmlRequest {
  fn to_pairs(&self) -> Vec<(&str, &str)> {
    let mut query = Vec::new();
    query.push(("tex", self.tex.as_str()));
    query.push(("preamble", self.preamble.as_str()));
    query.push(("comments", self.comments.as_str()));
    query.push(("post", self.post.as_str()));
    query.push(("timeout", self.timeout.as_str()));
    query.push(("format", self.format.as_str()));
    query.push(("whatsin", self.whatsin.as_str()));
    query.push(("whatsout", self.whatsout.as_str()));
    query.push(("pmml", self.pmml.as_str()));
    query.push(("cmml", self.cmml.as_str()));
    query.push(("mathtex", self.mathtex.as_str()));
    query.push(("nodefaultresources", &self.nodefaultresources));
    for p in self.preload.iter() {
      query.push(("preload", &p));
    }
    query
  }
}

fn latexml_call(params: Json<LatexmlRequest>) -> LatexmlResponse {
  let client = reqwest::Client::new();
  let mut res = client
    .post("http://127.0.0.1:8080/convert")
    .form(&params.to_pairs())
    .send()
    .unwrap();
  let latexml_res: LatexmlResponse = res.json().unwrap();
  latexml_res
}

fn llamapun_text_indexes(xml: &str) -> (Vec<String>, Vec<u64>) {
  let corpus_placeholder: Corpus = Corpus {
    path: "/tmp".to_string(),
    ..Corpus::default()
  };
  let mut document = Document {
    path: "/tmp".to_string(),
    dom: corpus_placeholder.html_parser.parse_string(xml).unwrap(),
    corpus: &corpus_placeholder,
    dnm: None,
  };
  let mut context = Context::new(&document.dom).unwrap();
  let mut words: Vec<String> = Vec::new();
  let mut word_indexes: Vec<u64> = Vec::new();

  // use only the first paragraph for this demo
  if let Some(mut paragraph) = document.paragraph_iter().next() {
    // we need to tokenize, fish out math lexemes, and map each word to its numeric index (or drop
    // if unknown)
    'sentences: for mut sentence in paragraph.iter() {
      for word in sentence.simple_iter() {
        if !word.range.is_empty() {
          let word_string =
            match data_helpers::ams_normalize_word_range(&word.range, &mut context, false) {
              Ok(w) => w,
              Err(_) => {
                break 'sentences;
              },
            };
          for lexeme in word_string.split(' ') {
            // if word is in the dictionary, record its index
            if let Some(idx) = DICTIONARY.lock().unwrap().get(lexeme) {
              // println!("{}: {}", lexeme, idx);
              words.push(lexeme.to_string());
              word_indexes.push(*idx);
            }
          }
        }
      }
    }
    // println!("Words: {:?}", words);
    // println!("Word count: {:?}", words.len());
  }
  (words, word_indexes)
}

#[derive(Debug, Clone, Serialize)]
pub struct Classification {
  r#abstract: f32,
  acknowledgement: f32,
  conclusion: f32,
  definition: f32,
  example: f32,
  introduction: f32,
  keywords: f32,
  proof: f32,
  proposition: f32,
  problem: f32,
  related_work: f32,
  remark: f32,
  result: f32,
}

impl From<Tensor<f32>> for Classification {
  fn from(t: Tensor<f32>) -> Classification {
    Classification {
      r#abstract: t[0],
      acknowledgement: t[1],
      conclusion: t[2],
      definition: t[3],
      example: t[4],
      introduction: t[5],
      keywords: t[6],
      proof: t[7],
      proposition: t[8],
      problem: t[9],
      related_work: t[10],
      remark: t[11],
      result: t[12],
    }
  }
}

#[derive(Debug, Serialize)]
struct Benchmark {
  latexml: u128,
  llamapun: u128,
  tensorflow: u128,
  total: u128,
}

#[derive(Debug, Serialize)]
struct ClassificationResponse {
  latexml: Option<LatexmlResponse>,
  classification: Option<Classification>,
  plaintext: Option<String>,
  embedding: Option<Vec<u64>>,
  benchmark: Benchmark,
}

fn pad_indexes(mut indexes: Vec<u64>) -> Vec<u64> {
  indexes.truncate(PARAGRAPH_SIZE);
  let padding = PARAGRAPH_SIZE - indexes.len();
  if padding > 0 {
    for _ in 0..padding {
      indexes.push(0);
    }
  }
  indexes
}

fn classify(session: State<Session>, indexes: Vec<u64>) -> Classification {
  println!("Session created");
  // println!("will classify: {:?}", indexes);

  // Grab the data out of the session.
  let indexes_f32: Vec<f32> = indexes.iter().map(|element| *element as f32).collect();
  let input_tensor = Tensor::new(&[1, PARAGRAPH_SIZE as u64])
    .with_values(indexes_f32.as_slice())
    .unwrap();
  let mut output_step = SessionRunArgs::new();

  let op_embed = TF_GRAPH
    .operation_by_name_required("embedding_1_input")
    .unwrap();
  let op_softmax = TF_GRAPH
    .operation_by_name_required("dense_1/Softmax")
    .unwrap();
  output_step.add_feed(&op_embed, 0, &input_tensor);
  println!("feed added.");

  let softmax_fetch_token = output_step.request_fetch(&op_softmax, 0);
  println!("softmax requested. running session");

  session.run(&mut output_step).unwrap();

  println!("session run completed. Obtaining prediction.");
  // Check our results.
  let prediction: Tensor<f32> = output_step.fetch(softmax_fetch_token).unwrap();

  let prediction_classification: Classification = prediction.into();
  prediction_classification
}

#[post("/process", format = "application/json", data = "<req>")]
fn process(session: State<Session>, req: Json<LatexmlRequest>) -> content::Json<String> {
  let start = Instant::now();
  // 1. obtain HTML5 via latexml
  let mut res = ClassificationResponse {
    latexml: None,
    benchmark: Benchmark {
      latexml: 0,
      llamapun: 0,
      tensorflow: 0,
      total: 0,
    },
    classification: None,
    plaintext: None,
    embedding: None,
  };
  let latexml_start = Instant::now();
  let latexml_response = latexml_call(req);
  res.benchmark.latexml = latexml_start.elapsed().as_millis();
  // 2. obtain word indexes of the first paragraph, via llamapun
  let llamapun_start = Instant::now();
  let (words, word_indexes) = llamapun_text_indexes(&latexml_response.result);
  res.latexml = Some(latexml_response);
  res.benchmark.llamapun = llamapun_start.elapsed().as_millis();
  // 3. obtain classification prediction via tensorflow
  let tensorflow_start = Instant::now();
  res.plaintext = Some(words.join(" "));
  let padded_indexes = pad_indexes(word_indexes);
  res.embedding = Some(padded_indexes.clone());
  res.classification = Some(classify(session, padded_indexes));
  res.benchmark.tensorflow = tensorflow_start.elapsed().as_millis();
  res.benchmark.total = start.elapsed().as_millis();
  // 4. package and respond
  content::Json(serde_json::to_string(&res).unwrap())
}

fn rocket() -> rocket::Rocket {
  rocket::ignite()
    .mount("/", routes![root, favicon, files, process])
    .attach(Template::fairing())
    .attach(CORS())
}

fn main() {
  // preload global statics
  let preload = Instant::now();
  println!("-- loading word dictionary");
  assert_eq!(DICTIONARY.lock().unwrap().get("NUM"), Some(&1));

  println!("-- instantiating TensorFlow graph");
  assert!(TF_GRAPH.graph_def().is_ok());

  // Crucially use the SAME tensorflow session throughout the lifetime of the server,
  // as instantiating a new session implies graph-initialization costs (~15 seconds) on the first
  // .run call. Reusing an already initialized session drops the 15 second overhead, and a
  // classification call becomes ~0.2 seconds
  let session = Session::new(&SessionOptions::new(), &TF_GRAPH).unwrap();

  println!("-- initializing llamapun globals");
  llamapun_text_indexes(
    "<html><body><div class=\"ltx_para\"><p class=\"ltx_p\">mock</p></div></body></html>",
  );
  println!(
    "-- preloading completed in {} seconds.",
    preload.elapsed().as_secs()
  );
  println!("-- launching Rocket web service");
  rocket().manage(session).launch();
}
