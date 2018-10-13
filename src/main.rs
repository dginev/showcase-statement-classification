#![feature(plugin)]
#![plugin(rocket_codegen)]

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate lazy_static;
extern crate hyper;
extern crate hyper_tls;
extern crate rocket;
extern crate rocket_contrib;
extern crate serde_json;
extern crate url;
extern crate tokio_core;
extern crate llamapun;
extern crate libxml;
extern crate regex;
// NOTE! Expectation is tensorflow 1.10.1 at the moment, and there is no end of potential grief if there is a mismatch.
extern crate tensorflow;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::str;
use std::sync::Mutex;
// use std::cell::RefCell;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::result::Result;

use libxml::xpath::Context;
use llamapun::data::{Corpus, Document};
use llamapun::dnm;
use regex::Regex;

// use tensorflow::Code;
use tensorflow::{Tensor,Graph,Session,SessionOptions, SessionRunArgs};
use tensorflow::ImportGraphDefOptions;

use hyper::header::{HeaderValue, CONTENT_LENGTH, CONTENT_TYPE};
use hyper::rt::{Future, Stream};
use hyper::{Body, Method};
use hyper::{Client, Request};
use hyper::client::{HttpConnector};
use hyper_tls::HttpsConnector;
use tokio_core::reactor::Core;

use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Header;
use rocket::response::content;
use rocket::response::status::{NotFound};
use rocket::response::{NamedFile};

use rocket_contrib::{Json, Template};

// We need a singleton Corpus object to hold the various expensive state objects
static MAX_WORD_LENGTH : usize = 25;
static PARAGRAPH_SIZE : usize = 480;

lazy_static! {
  static ref IS_NUMERIC : Regex = Regex::new(r"^-?(?:\d+)(?:[a-k]|(?:\.\d+(?:[eE][+-]?\d+)?))?$").unwrap();
  static ref DICTIONARY : Mutex<HashMap<String, u64>> = {
    let json_file = File::open(Path::new("ams_word_index.json")).expect("file not found");
    let dictionary: HashMap<String, u64> = serde_json::from_reader(json_file).expect("error while reading json");
    Mutex::new(dictionary)
  };
  // static ref MODEL: RefCell<Session> = { 
  //   let mut graph = Graph::new();
  //   let session = Session::from_saved_model(&SessionOptions::new(), 
  //                                               &["serve"],
  //                                               &mut graph,
  //                                               "model.h5").unwrap();
  //   RefCell::new(session)
  // };
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
  log: String
}

impl LatexmlRequest {
  fn to_query(&self) -> String {
    let mut query = format!("tex={}&preamble={}&comments={}&post={}&timeout={}&format={}&whatsin={}&whatsout={}&pmml={}&cmml={}&mathtex={}&mathlex={}&nodefaultresources={}",
      uri_esc(&self.tex), uri_esc(&self.preamble), uri_esc(&self.comments), 
      uri_esc(&self.post), uri_esc(&self.timeout), uri_esc(&self.format),
       uri_esc(&self.whatsin), uri_esc(&self.whatsout), uri_esc(&self.pmml), 
       uri_esc(&self.cmml), uri_esc(&self.mathtex), uri_esc(&self.mathlex), uri_esc(&self.nodefaultresources));
    for p in self.preload.iter() {
      query.push('&');
      query.push_str("preload=");
      query.push_str(&uri_esc(&p));
    }
    query
  }
}

fn uri_esc(param: &str) -> String {
  let mut param_encoded: String =
    url::percent_encoding::utf8_percent_encode(param, url::percent_encoding::DEFAULT_ENCODE_SET)
      .collect::<String>();
  // TODO: This could/should be done faster by using lazy_static!
  for &(original, replacement) in &[
    (":", "%3A"),
    ("/", "%2F"),
    ("\\", "%5C"),
    ("$", "%24"),
    (".", "%2E"),
    ("!", "%21"),
    ("@", "%40"),
  ] {
    param_encoded = param_encoded.replace(original, replacement);
  }
  // if param_pure != param_encoded {
  //   println!("Encoded {} to {:?}", param_pure, param_encoded);
  // } else {
  //   println!("No encoding needed: {:?}", param_pure);
  // }
  param_encoded
}

fn latexml_call(params: Json<LatexmlRequest>) -> LatexmlResponse {
  let payload = params.into_inner().to_query();
  let payload_len = payload.len();

  let client : Client<HttpsConnector<HttpConnector>> = Client::builder()
    .build::<_, hyper::Body>(HttpsConnector::new(4).expect("TLS initialization failed"));

  let url: hyper::Uri = "https://latexml.mathweb.org/convert".parse().unwrap();

  let mut req = Request::builder()
    .uri(url)
    .method(Method::POST)
    .body(Body::from(payload))
    .unwrap();

  req
    .headers_mut()
    .insert(CONTENT_TYPE, HeaderValue::from_static("application/x-www-form-urlencoded"));
  req.headers_mut().insert(
    CONTENT_LENGTH,
    HeaderValue::from_str(&payload_len.to_string()).unwrap(),
  );

  let mut res_data : Vec<u8> = Vec::new();
  let mut core = Core::new().unwrap();
  {
    let work = client.request(req).and_then(|res| {
      res.into_body().for_each(|chunk| {
        res_data.extend_from_slice(&*chunk.into_bytes());
        Ok(())
      })
    });
    core.run(work).unwrap();
  }
  serde_json::from_str(
    str::from_utf8(res_data.as_slice()).unwrap_or("")
  ).unwrap()
}

fn llamapun_text_indexes(xml: &str) -> Vec<f32> {
  let corpus_placeholder : Corpus = Corpus {
    path: "/tmp".to_string(),
    ..Corpus::default()
  };
  let mut document = Document {
    path: "/tmp".to_string(),
    dom: corpus_placeholder.html_parser.parse_string(xml).unwrap(),
    corpus: &corpus_placeholder,
    dnm: None
  };
  let mut context = Context::new(&document.dom).unwrap();
  let mut words : Vec<f32> = Vec::new();

  // use only the first paragraph for this demo
  if let Some(mut paragraph) = document.paragraph_iter().next() {
    // we need to tokenize, fish out math lexemes, and map each word to its numeric index (or drop if unknown)
    'sentences: for mut sentence in paragraph.iter() {
      for word in sentence.simple_iter() {
        if !word.range.is_empty() {
          let mut word_string = word
            .range
            .get_plaintext()
            .chars()
            .filter(|c| c.is_alphanumeric()) // drop apostrophes, other noise?
            .collect::<String>()
            .to_lowercase();
          if word_string.len() > MAX_WORD_LENGTH {
            // Using a more aggressive normalization, large words tend to be conversion
            // errors with lost whitespace - drop the entire paragraph when this occurs.
            break 'sentences;
          }
          let mut word_str: &str = &word_string;
          // Note: the formula and citation counts are an approximate lower bound, as
          // sometimes they are not cleanly tokenized, e.g. $k$-dimensional
          // will be the word string "mathformula-dimensional"
          if word_string.contains("mathformula") {
            for lexeme in dnm::node::lexematize_math(word.range.get_node(), &mut context).split(" ") {
              if !lexeme.is_empty() {
                // if word is in the dictionary, record its index
                if let Some(idx) = DICTIONARY.lock().unwrap().get(lexeme) {
                  // println!("{}: {}", lexeme, idx);
                  words.push(*idx as f32);
                }
              }
            }
            word_str = "";
          } else if word_string.contains("citationelement") {
            word_str = "citationelement";
          } else if IS_NUMERIC.is_match(&word_string) {
            word_str = "NUM";
          }

          if !word_str.is_empty() {
            // if word is in the dictionary, record its index
            if let Some(idx) = DICTIONARY.lock().unwrap().get(word_str) {
              // println!("{}: {}", word_str, idx);
              words.push(*idx as f32);
            }
          }
        }
      }
    }
    // println!("Words: {:?}", words);
    // println!("Word count: {:?}", words.len());
  }
  words
}

fn classify(mut indexes: Vec<f32>) -> Result<(),Box<Error>> {
  indexes.truncate(PARAGRAPH_SIZE);
  let padding = PARAGRAPH_SIZE - indexes.len();
  if padding > 0 {
    for _ in 0..padding {
      indexes.push(0.0);
    }
  }
  
  // Load the computation graph defined by regression.py.
  let filename = "model.pb";
  println!("loading TF model...");
  let mut graph = Graph::new();
  let mut proto = Vec::new();
  File::open(filename)?.read_to_end(&mut proto)?;
  println!("read in graph data...");
  graph.import_graph_def(&proto, &ImportGraphDefOptions::new())?;
  println!("Graph imported");
  let mut session = Session::new(&SessionOptions::new(), &graph)?;
  println!("Session created");

  // Grab the data out of the session.
  let input_tensor = Tensor::new(&[1,480]).with_values(indexes.as_slice())?;
  let mut output_step = SessionRunArgs::new();

  let op_embed = graph.operation_by_name_required("embedding_1_input")?;
  let op_softmax = graph.operation_by_name_required("dense_1/Softmax")?;
  output_step.add_feed(&op_embed, 0, &input_tensor);
  println!("feed added.");

  let softmax_fetch_token = output_step.request_fetch(&op_softmax, 0);
  println!("sofmtax requested. running session");

  session.run(&mut output_step)?;

  println!("session run completed. Obtaining prediction.");
  // Check our results.
  let prediction : Tensor<f32>  = output_step.fetch(softmax_fetch_token)?;
  println!("prediction: {:?}", prediction);
  for p in prediction.iter() {
    println!("val: {:?}", p);
  }

  Ok(())
}

#[post("/convert", format = "application/json", data = "<req>")]
fn convert(req: Json<LatexmlRequest>) -> content::Json<String> { 
  // 1. obtain HTML5 via latexml
  let latexml_response = latexml_call(req);
  // 2. obtain word indexes of the first paragraph, via llamapun
  let word_indexes = llamapun_text_indexes(&latexml_response.result);
  // 3. obtain classification prediction via tensorflow
  match classify(word_indexes) {
  Ok(prediction) => println!("ok prediction: {:?}", prediction),
   Err(e) => println!("error! {:?}", e)
  };
  // 4. package and respond
  // content::Json(serde_json::serialize());
  content::Json(serde_json::to_string(&latexml_response).unwrap())
}

fn rocket() -> rocket::Rocket {
  rocket::ignite()
    .mount("/", routes![root, favicon, files, convert])
    .attach(Template::fairing())
    .attach(CORS())
}

fn main() {
  // preload static
  assert_eq!(DICTIONARY.lock().unwrap().get("NUM"), Some(&1));
  rocket().launch(); 
}
