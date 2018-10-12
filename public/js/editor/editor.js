var examples = load_examples();

function show_log() {
  $('#onthefly').hide();
  $('#log').show();
}
function show_result() {
  $('#log').hide();
  $('#onthefly').show();

  if (!canMathML && typeof MathJax !== "undefined") {
    MathJax
      .Hub
      .Typeset();
  }
}
function setup_message(data) {
  $('#message').html($('#message').text(data.status).html().replace(/\n/g, "<br />") + "<a href='#'>(Details)</a>");
  $('#message').hover(function () {
    show_log();
  }, function () {
    show_result();
  });
  $('#message').click(function () {
    $('#message')
      .unbind('mouseenter')
      .unbind('mouseleave');
    show_log();
  });
  $('#log').hide();
  $('#log').html($('#log').text(data.log).html().replace(/\n/g, "<br />"));
}

$.urlParam = function (name) {
  var results = new RegExp('[\?&]' + name + '=([^&#]*)').exec(window.location.href);
  if (results == null) {
    return false;
  } else {
    return results[1] || false;
  }
}

var ac_counter = 0;
var send_called = 0;
var mouse_pressed = 0;
var timeout = null;
var hasFatal = /fatal error/;
var hasPreamble = /^([\s\S]*\\begin{document})([\s\S]*)\\end{document}([\s\S]*)$/;

var sendRequest = function (tex, my_counter, onthefly) {
  if (my_counter == ac_counter) {
    $('#log').html('');
    $('#previewtext').html('Converting...');
    $('#message').html('Converting...');
    $("body").css("cursor", "progress");
    if (ac_counter == 1)
      send_called = 0;
    send_called++;
    $('#counter').html(send_called);
    //Check if preamble exists:
    var m = hasPreamble.exec(tex);
    var preamble = null;
    if (m != null) {
      preamble = "literal:" + m[1];
      tex = m[2];
    }
    $.ajax({
      type: "POST",
      url: "/convert",
      contentType: 'application/json',
      data: JSON.stringify({ // excplicitly unroll the fragment-html profile, as we want to add the math lexemes output on top
        "tex": tex || "",
        "preamble": preamble || "",
        "comments": "",
        "post": "",
        "timeout": "120",
        "format": "html5",
        "whatsin": "fragment",
        "whatsout": "fragment",
        "pmml": "",
        "cmml": "",
        "mathtex": "",
        "mathlex": "",
        "nodefaultresources": "",
        "preload": ["LaTeX.pool", "article.cls", "amsmath.sty", "amsthm.sty", "amstext.sty", "amssymb.sty", "eucal.sty", "[dvipsnames]xcolor.sty", "url.sty", "hyperref.sty", "[ids]latexml.sty", "llamapun.sty"]
      }),
    }).done(function (data) {
      console.log("success: ", data);
      setup_message(data);
      if (onthefly) {
        if (!hasFatal.test(data.status)) {
          if ((data.result != '') && (my_counter <= ac_counter)) {
            $('#onthefly').html(data.result);
            show_result();
          }
        } else {
          show_log();
        }
      }
      $('#previewtext').text('On-the-Fly Preview');
      $("body").css("cursor", "auto");
    });
  }
}

function do_convert_on_the_fly(e) {
  if (e) {
    var key = e.keyCode;
    if (!key)
      key = 0;
  }
  else {
    var key = 0;
  }

  ac_counter++;
  if (((key < 37 || key > 40) && key > 32 && key <= 250) || key == 8 || key == 0) {
    // immediately cancel outstanding requests
    if (timeout)
      clearTimeout(timeout);
    ac_counter--;
    var tex = $("#editor").val();
    if (!tex) {
      ac_counter = 0;
      $('#onthefly').html(' ');
      return;
    }

    timeout = setTimeout(function () {
      console.log("Sending tex: ", tex);
      sendRequest(tex, ac_counter, true)
    }, 300);
  }
}

function editor_conversion_start() {
  setTimeout(do_convert_on_the_fly, 100);
  show_result();
}

function example_select_handler() {
  option = $('#example_select option:selected').first();
  var example_requested = option && option.attr("value");

  if (example_requested) {
    $('#onthefly').html('');
    $("#editor").val(examples[example_requested]);
  }
}


$('#ltxstyle_select').change(function () {
  var stylename = "";
  $('#ltxstyle_select option:selected').each(function () {
    stylename = $(this).attr("value");
  });
  if (stylename.length > 0) {
    // Dynamically load the CSS:
    $('#ltxstyle_link').remove();
    $("<link>")
      .appendTo("head")
      .attr({
        rel: 'stylesheet',
        type: 'text/css',
        id: 'ltxstyle_link',
        href: 'css/external/' + stylename + '.css'
      });
  }
});

var tex_requested = $.urlParam('tex');
if (tex_requested) {
  $("#editor").val(decodeURIComponent(tex_requested));
} else {
  $("#editor").val("Write a LaTeX paragraph here. Let $e=mc^2$ be a preliminary for the following ...");
}

$("#editor").on('change', function () {
  editor_conversion_start();
});
editor_conversion_start();