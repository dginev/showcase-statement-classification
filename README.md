# Showcase for 13-class scientific statement classification

### Preview available at:
https://corpora.mathweb.org/classify_paragraph

### Method
 * [latexml](https://github.com/brucemiller/LaTeXML) converts the source into an HTML5 document
 * [llamapun](https://github.com/KWARC/llamapun) tokenizes the first paragraph into a plain-text representation with sub-formula lexemes
 * [tensorflow](https://github.com/tensorflow/rust) executes a pre-trained BiLSTM model to obtain the likelihoods of 6 potential amsthm classes
 * served as a [rocket](https://rocket.rs/) web service
 
### Details

The current deployed model is a Keras **BiLSTM(128)→BiLSTM(64)→LSTM(64)**, with a **Dense(13)** softmax output.

The input layer is embedded via the [arxmliv 08.2018 GloVe embeddings](https://sigmathling.kwarc.info/resources/arxmliv-embeddings-082018/), as well as padded/truncated to a maximum length of 480 words. 
A paragraph is hence a fixed `(480,300)` matrix, as passed into the bilstm layer.

The specific model in this demo was trained on **8.3 million** paragraphs from the [arxmliv 08.2018 dataset](https://sigmathling.kwarc.info/resources/arxmliv-dataset-082018/),
and tested on **2.1 million paragraphs** respectively, obtaining a **0.91 F1 score** on a target of 13 classes.
The base rate baseline was 0.38, the frequency of the "proposition" class.

For more experimental details, please see the main [experiment repository](https://github.com/dginev/arxiv-statement-classification).

For practical evaluation, a likelihood threshold could be used, where entries with smaller likelihoods can be considered as an "other" label.

### Notes

*Pending publication*

The server is running a CPU version of the underling `CuDNNLSTM` layers, so expect rather slow runtimes, of 15-20 seconds for the tensorflow stage. It is also not dedicated to serving this showcase, so performance may additionally vary.
