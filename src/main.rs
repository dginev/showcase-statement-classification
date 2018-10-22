#![feature(plugin)]
#![feature(duration_as_u128)] 
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
use std::time::Instant;

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
static INDEX_FROM : f32 = 2.0; // arxiv.py preprocessing offsets all indexes with 2, e.g. NUM 1 --> 3

lazy_static! {
  static ref IS_NUMERIC : Regex = Regex::new(r"^-?(?:\d+)(?:[a-k]|(?:\.\d+(?:[eE][+-]?\d+)?))?$").unwrap();
  static ref DICTIONARY : Mutex<HashMap<String, u64>> = {
    let json_file = File::open(Path::new("ams_word_index.json")).expect("file not found");
    let dictionary: HashMap<String, u64> = serde_json::from_reader(json_file).expect("error while reading json");
    Mutex::new(dictionary)
  };
  static ref TF_GRAPH : Graph = {
    // Load the computation graph defined by regression.py.
    let filename = "v3_model_cat8_cpu.pb";
    println!("-- loading TF model");
    let mut graph = Graph::new();
    let mut proto = Vec::new();
    File::open(filename).unwrap().read_to_end(&mut proto).unwrap();
    println!("-- reading in graph data");
    graph.import_graph_def(&proto, &ImportGraphDefOptions::new()).unwrap();
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
                  words.push(INDEX_FROM+(*idx as f32));
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
              words.push(INDEX_FROM+(*idx as f32));
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

// Version 2:
// #[derive(Debug, Serialize)]
// struct Classification {
//   acknowledgement: f32,
//   // algorithm: f32,
//   // caption: f32,
//   definition: f32, 
//   example: f32,
//   theorem: f32,
//   problem: f32,
//   proof: f32, 
// }


// impl From<Tensor<f32>> for Classification {
//   fn from(t: Tensor<f32>) -> Classification {
//     Classification {
//       acknowledgement: t[0],
//       //algorithm: t[1],
//       // caption: t[0],
//       definition: t[1],
//       example: t[2],
//       theorem: t[3],
//       problem: t[4],
//       proof: t[5],
//     }
//   }
// }

// Version 3:
#[derive(Debug, Serialize)]
struct Classification {
  acknowledgement: f32,
  proposition: f32,
  definition: f32, 
  example: f32,
  introduction: f32,
  problem: f32,
  proof: f32, 
  related_work: f32,
}


impl From<Tensor<f32>> for Classification {
  fn from(t: Tensor<f32>) -> Classification {
    Classification {
      acknowledgement: t[0],
      proposition: t[1],
      definition: t[2],
      example: t[3],
      introduction: t[4],
      problem: t[5],
      proof: t[6],
      related_work: t[7]
    }
  }
}

#[derive(Debug, Serialize)]
struct Benchmark {
  latexml: u128,
  llamapun: u128,
  tensorflow_cpu: u128,
  total: u128,
}

#[derive(Debug, Serialize)]
struct ClassificationResponse {
  latexml: Option<LatexmlResponse>,
  classification: Option<Classification>,
  benchmark: Benchmark,
}


fn classify(mut indexes: Vec<f32>) -> Result<Classification,Box<Error>> {
  indexes.truncate(PARAGRAPH_SIZE);
  let padding = PARAGRAPH_SIZE - indexes.len();
  if padding > 0 {
    for _ in 0..padding {
      indexes.push(0.0);
    }
  }
  
  let mut session = Session::new(&SessionOptions::new(), &TF_GRAPH)?;
  println!("Session created");

  // Grab the data out of the session.
  let input_tensor = Tensor::new(&[1,480]).with_values(indexes.as_slice())?;
  let mut output_step = SessionRunArgs::new();

  let op_embed = TF_GRAPH.operation_by_name_required("embedding_1_input")?;
  let op_softmax = TF_GRAPH.operation_by_name_required("dense_1/Softmax")?;
  output_step.add_feed(&op_embed, 0, &input_tensor);
  println!("feed added.");

  let softmax_fetch_token = output_step.request_fetch(&op_softmax, 0);
  println!("sofmtax requested. running session");

  session.run(&mut output_step)?;

  println!("session run completed. Obtaining prediction.");
  // Check our results.
  let prediction : Tensor<f32>  = output_step.fetch(softmax_fetch_token)?;
  
  Ok(prediction.into())
}

#[post("/process", format = "application/json", data = "<req>")]
fn process(req: Json<LatexmlRequest>) -> content::Json<String> { 
  let start = Instant::now();
  // 1. obtain HTML5 via latexml
  let mut res = ClassificationResponse {
    latexml: None,
    benchmark: Benchmark { latexml: 0, llamapun: 0, tensorflow_cpu: 0, total: 0},
    classification: None,
  };
  let latexml_start = Instant::now();
  let latexml_response = latexml_call(req);
  res.benchmark.latexml = latexml_start.elapsed().as_millis();
  // 2. obtain word indexes of the first paragraph, via llamapun
  let llamapun_start = Instant::now();
  let word_indexes = llamapun_text_indexes(&latexml_response.result);
  res.latexml = Some(latexml_response);
  res.benchmark.llamapun = llamapun_start.elapsed().as_millis();
  // 3. obtain classification prediction via tensorflow
  let tensorflow_start = Instant::now();
  match classify(word_indexes) { 
    Ok(prediction) => {res.classification = Some(prediction);},
    Err(e) => println!("classification failed: {:?}", e)
  };
  res.benchmark.tensorflow_cpu = tensorflow_start.elapsed().as_millis();
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

  println!("-- initializing llamapun globals");
  llamapun_text_indexes("<html><body><div class=\"ltx_para\"><p class=\"ltx_p\">mock</p></div></body></html>");
  println!("-- preloading completed in {} seconds.",preload.elapsed().as_secs());
  println!("-- launching Rocket web service");
  rocket().launch(); 
}
