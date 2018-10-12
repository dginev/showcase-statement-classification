#![feature(plugin)]
#![plugin(rocket_codegen)]

#[macro_use]
extern crate serde_derive;
extern crate hyper;
extern crate hyper_tls;
extern crate rocket;
extern crate rocket_contrib;
extern crate serde_json;
extern crate tokio_core;

use std::collections::HashMap;
use std::fs::File;
use std::io::Cursor;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str;

use hyper::header::{HeaderValue, CONTENT_LENGTH, CONTENT_TYPE};
use hyper::rt::{Future, Stream};
use hyper::{Body, Method};
use hyper::{Client, Request};
use hyper_tls::HttpsConnector;
use tokio_core::reactor::Core;

use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Header;
use rocket::response::content;
use rocket::response::status::{Accepted, NotFound};
use rocket::response::{NamedFile, Redirect};

use rocket_contrib::{Json, Template};

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

fn latexml_call(params: Json<LatexmlRequest>) -> content::Json<String> {
  let json_str = format!("{:?}", params);
  let json_str_len = json_str.len();
  let mut core = Core::new().unwrap();
  let handle = core.handle();

  let client = Client::builder()
    .build::<_, hyper::Body>(HttpsConnector::new(4).expect("TLS initialization failed"));
  // .build(&handle);

  let url_with_query: hyper::Uri = "https://latexml.mathweb.org/convert"
    .to_string()
    .parse()
    .unwrap();

  let mut req = Request::builder()
    .uri(url_with_query)
    .method(Method::POST)
    .body(Body::from(json_str))
    .unwrap();

  req
    .headers_mut()
    .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
  req.headers_mut().insert(
    CONTENT_LENGTH,
    HeaderValue::from_str(&json_str_len.to_string()).unwrap(),
  );

  let post = client
    .request(req)
    .and_then(|res| res.into_body().concat2());
  let posted = match core.run(post) {
    Ok(posted_data) => match str::from_utf8(&posted_data) {
      Ok(posted_str) => posted_str.to_string(),
      Err(e) => {
        println!("err: {}", e);
        return content::Json("{ 'status': 'Fatal error in remote latexml request.' }".to_string());
      },
    },
    Err(e) => {
      println!("err: {}", e);
      return content::Json("{ 'status': 'Fatal error in remote latexml request.' }".to_string());
    },
  };

  content::Json(posted)
}

#[post("/convert", format = "application/json", data = "<req>")]
fn convert(req: Json<LatexmlRequest>) -> content::Json<String> {
  println!("req: {:?}", req);
  latexml_call(req)
}

fn rocket() -> rocket::Rocket {
  rocket::ignite()
    .mount("/", routes![root, favicon, files, convert])
    .attach(Template::fairing())
    .attach(CORS())
}

fn main() { rocket().launch(); }
