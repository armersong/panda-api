use actix_web::{http, web, HttpRequest, HttpResponse};
use actix_web::dev::ResourceDef;
use actix_files;

use actix_multipart::Multipart;
use futures::StreamExt;
use regex::Regex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value, Map};
use std::collections::HashMap;
use std::fs;
use std::io::prelude::*;
use std::sync::Mutex;
use log::debug;
use crate::db::{self, ApiDoc};


#[derive(Serialize, Deserialize, Debug)]
struct DocSummary {
    pub name: String,
    pub desc: String,
    pub order: i64,
    pub filename: String,
}


#[derive(Serialize, Deserialize, Debug)]
pub struct ApiDocDataRequest {
    filename: String,
}


pub async fn server_info() -> HttpResponse {
    HttpResponse::Ok().json(json!({
      "name": "mockrs",
      "author": "PrivateRookie"
    }))
}


/// 根据接口文件路径获取接口文档详情
pub async fn get_api_doc_data(req: HttpRequest, req_get: web::Query<ApiDocDataRequest>, data: web::Data<Mutex<db::Database>>) -> HttpResponse {
    let mut data = data.lock().unwrap();
    let mut api_docs = &data.api_docs;

    for (_, doc) in api_docs {
        if doc.filename == req_get.filename {
            let mut apis = Vec::new();
            for api in &doc.apis {
                let api = api.lock().unwrap();
                apis.push({ &*api }.clone());
            }
            return HttpResponse::Ok().json(
                json!({
                    "name": doc.name,
                    "desc": doc.desc,
                    "order": doc.order,
                    "filename": doc.filename,
                    "apis": apis}));
        }
    }

    HttpResponse::Ok().json(json!({
      "code": -1,
      "msg": "没有该接口文档文件"
    }))
}


/// 获取项目接口的基本信息
/// 返回项目名称，介绍，项目接口简要列表
/// 前端需要自己根据 api_doc 的order进行排序
pub async fn get_api_doc_basic(req: HttpRequest, data: web::Data<Mutex<db::Database>>) -> HttpResponse {
    let mut data = data.lock().unwrap();
    let mut basic_data = &data.basic_data;
    let mut api_docs = &data.api_docs;

    let mut docs = Vec::new();
    for (_, doc) in api_docs {
        docs.push(DocSummary { name: doc.name.clone(), desc: doc.desc.clone(), order: doc.order, filename: doc.filename.clone() });
    }

    HttpResponse::Ok().json(json!({
      "project_name": &basic_data.project_name,
      "project_desc": &basic_data.project_desc,
      "read_me": &basic_data.read_me,
      "api_docs": docs
    }))
}


/// 获取_data目录中的数据, models数据 或者其它加载数据
pub async fn get_api_doc_schema_data(req: HttpRequest, req_get: web::Query<ApiDocDataRequest>) -> HttpResponse {
    let read_me = match fs::read_to_string(&req_get.filename) {
        Ok(x) => x,
        Err(_) => "no data file".to_string()
    };

    HttpResponse::Ok().content_type("application/json").body(read_me)
}


#[derive(Deserialize, Debug)]
pub struct FormData {
    username: String,
}

/// 处理post、put、delete 请求
///
pub async fn action_handle(req: HttpRequest, request_body: Option<web::Json<Value>>, request_query: Option<web::Query<Value>>, request_form_data: Option<Multipart>, db_data: web::Data<Mutex<db::Database>>) -> HttpResponse {

    // for api documents homepage
    let req_path = req.path();
    let body_mode = get_request_body_mode(&req);

    if req_path == "/" {
        let d = match fs::read_to_string("theme/index.html") {
            Ok(x) => x,
            Err(_) => "no data file".to_string()
        };
        return HttpResponse::Ok().content_type("text/html").body(d);
    }


    let mut new_request_body;
    if &body_mode == "form-data" {
        // 没有request_body，有可能是文件上传
        // 进行文件上传处理

        let mut form_data: Map<String, Value> = Map::new();
        if let Some(mut payload) = request_form_data {
            // 如果是文件上传
            while let Some(item) = payload.next().await {
                if let Ok(mut field) = item {
                    let content_type = match field.content_disposition() {
                        Some(v) => v,
                        None => {
                            break;
                        }
                    };
                    let x = field.headers().clone();
                    let x = x.get("content-disposition").unwrap().to_str().unwrap();
                    let re = Regex::new(r#"form-data; name="\w+""#).unwrap();

                    let mut field_name = "";
                    if let Some(m) = re.find(x) {
                        field_name = &x[m.start() + 17..m.end() - 1];
                    };

                    let mut filename = "";
                    if let Some(f) = content_type.get_filename() {
                        filename = f;
                    }

                    match std::fs::create_dir_all("./_data/_upload") {
                        Ok(i) => (),
                        Err(e) => ()
                    }

                    let filepath = format!("./_data/_upload/{}", filename);
                    let filepath2 = &format!("./_data/_upload/{}", filename);

                    if let Ok(mut f) = web::block(|| std::fs::File::create(filepath)).await {
                        while let Some(chunk) = field.next().await {
                            let data = chunk.unwrap();

                            if let Ok(x) = f.write_all(&data) {
                                form_data.insert(field_name.to_string(), Value::String(filename.to_string()));
                                form_data.insert(format!("__{}", field_name), Value::String(format!("/_upload/{}", filename)));
                            } else {
                                println!("create file error {}", filepath2);
                            }
                        }
                    } else {
                        let x = field.next();
                        while let Some(chunk) = field.next().await {
                            let data = chunk.unwrap();
                            let x = data.to_vec();
                            let v = std::str::from_utf8(&x).unwrap();
                            form_data.insert(field_name.to_string(), Value::String(v.to_string()));
                        }

                    }
                    continue;
                }
                break;
            }
        }
        new_request_body = Value::Object(form_data);
    } else {
        new_request_body = match request_body {
            Some(x) => {
                x.into_inner()
            }
            None => Value::Null
        };
    }

    let request_query = match request_query {
        Some(x) => x.into_inner(),
        None => Value::Null
    };

    find_response_data(&req, new_request_body, request_query, db_data)
}




/// 找到对应url 对应请求的数据
///
fn find_response_data(req: &HttpRequest, request_body: Value, request_query: Value, db_data: web::Data<Mutex<db::Database>>) -> HttpResponse {
    let db_data = db_data.lock().unwrap();
    let api_data = &db_data.api_data;
    let req_path = req.path();
    let req_method = req.method().as_str();


    for (k, a_api_data) in api_data {
        // 匹配
        let res = ResourceDef::new(k);
        if res.is_match(req_path) {
            let a_api_data = match a_api_data.get(req_method) {
                Some(v) => v,
                None => {
                    return HttpResponse::Ok().json(json!({
                        "code": - 1,
                        "msg": format ! ("this api address {} not defined method {}", req_path, req_method)
                    }));
                }
            };
            let a_api_data = a_api_data.lock().unwrap();

            let test_data = &a_api_data.test_data;

            if test_data.is_null() {
                return HttpResponse::Ok().json(json!({
                    "code": - 1,
                    "msg": format ! ("this api {} with defined method {} have not test_data", req_path, req_method)
                }));
            }

            if !test_data.is_array() {
                return HttpResponse::Ok().json(json!({
                    "code": - 1,
                    "msg": format ! ("this api {} with defined method {} test_data is not a array", req_path, req_method)
                }));
            }

            let x = create_mock_response(&a_api_data.response);
            let test_data = test_data.as_array().unwrap();

            'a_loop: for test_case_data in test_data {
                let case_body = match test_case_data.get("body") {
                    Some(v) => v,
                    None => &Value::Null
                };
                let case_query = match test_case_data.get("query") {
                    Some(v) => v,
                    None => &Value::Null
                };
                let case_response = match test_case_data.get("response") {
                    Some(v) => v,
                    None => &Value::Null
                };

                if is_value_equal(&request_body, case_body) && is_value_equal(&request_query, case_query) {
                    return HttpResponse::Ok().json(case_response);
                }
            }
        }
    };

    HttpResponse::Ok().json(json!({
        "code": - 1,
        "msg": format ! ("this api address {} no test_data match", req_path)
    }))
}


/// 判断两个serde value的值是否相等
/// 只要value2中要求的每个字段，value1中都有，就表示相等, 也就是说value1的字段可能会比value2多
fn is_value_equal(value1: &Value, value2: &Value) -> bool {
    if value1.is_null() & &value2.is_null() {
        return true;
    }
    match value1 {
        Value::Object(value1_a) => {
            match value2.as_object() {
                Some(value2_a) => {
                    if value1_a.is_empty() & &value2_a.is_empty() {
                        return true;
                    }
                    for (k, v) in value2_a {
                        match value1_a.get(k) {
                            // 判断请求数据 与测试数据集的每个字段的值是否相等
                            Some(v2) => {
                                if v2 != v {
                                    return false;
                                }
                            }
                            None => {
                                return false;
                            }
                        }
                    }

                    return true;
                }
                None => {
                    if value1_a.is_empty() & &value2.is_null() {
                        return true;
                    }
                    return false;
                }
            }
        }
        Value::Array(a) => (),
        Value::Null => {
            // 让null 和 empty一样的相等
            match value2.as_object() {
                Some(value2_a) => {
                    if value2_a.is_empty() {
                        return true;
                    }
                }
                None => {
                    return false;
                }
            }
        }
        _ => {
            println!("Invalid Json Struct {:?}", value1);
        }
    }
    false
}



/// 获取请求的request_body
fn get_request_body_mode(req:&HttpRequest) -> String {
    let req_method = req.method().as_str();
    if req_method == "GET" {
        return "".to_string();
    }

    if let Some(head_value) = req.headers().get("content-type") {
        if let Ok(value_str) = head_value.to_str() {
            if value_str == "application/json" {
                return "json".to_string();
            } else if value_str.starts_with("multipart/form-data;") {
                return "form-data".to_string();
            } else if value_str == "text/plain" {
                return "text".to_string();
            } else if value_str == "application/javascript" {
                return "javascript".to_string();
            } else if value_str == "text/html" {
                return "html".to_string();
            } else if value_str == "application/xml" {
                return "xml".to_string();
            }
        }
    }

    "".to_string()
}



pub fn create_mock_response(response_model: &Value) -> Map<String, Value> {
    let mut result: Map<String, Value> = Map::new();
    if response_model.is_object() {
        let response_model = response_model.as_object().unwrap();
        for (key, value) in response_model {
            let field_type = match value.get("type") {
                Some(v) => v.as_str().unwrap(),
                None => "string"
            };

            match field_type {
                "number" => {}
                "string" | _ => {}
            }
        }
    }

    result
}