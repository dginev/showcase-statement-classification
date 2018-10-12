#![feature(plugin)]
#![plugin(rocket_codegen)]

#[macro_use]
extern crate serde_derive;
extern crate hyper;
extern crate hyper_tls;
extern crate rocket;
extern crate rocket_contrib;
extern crate serde_json;
extern crate url;
extern crate tokio_core;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::str;

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

fn latexml_call(params: Json<LatexmlRequest>) -> content::Json<String> {
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
      println!("Response: {}", res.status());
      println!("Headers: {:#?}", res.headers());
      println!("Body: {:?}", res);

      // The body is a stream, and for_each returns a new Future
      // when the stream is finished, and calls the closure on
      // each chunk of the body...
      res.into_body().for_each(|chunk| {
        res_data.extend_from_slice(&*chunk.into_bytes());
        Ok(())
      })
    });
    core.run(work).unwrap();
  }
  let res_string = String::from(str::from_utf8(res_data.as_slice()).unwrap_or(""));
  println!("body: {:?}", res_string);
  content::Json(res_string)
}

#[post("/convert", format = "application/json", data = "<req>")]
fn convert(req: Json<LatexmlRequest>) -> content::Json<String> { latexml_call(req) }

fn rocket() -> rocket::Rocket {
  rocket::ignite()
    .mount("/", routes![root, favicon, files, convert])
    .attach(Template::fairing())
    .attach(CORS())
}

fn main() { rocket().launch(); }
