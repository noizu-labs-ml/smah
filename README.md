README
======


[documentation](https://smah.readthedocs.io/en/latest/)

[git](https://github.com/noizu-labs-ml/smah)


SMAH the SMart As Hell SysteM Admin Helper.

Welcome to **SMAH** the **S**yste**M** **A**dmin **H**elper that just so happens to be
**SM**art **A**s **H**ell, an early prototype of a suite of tools designed 
to assist system administrators, data scientists, developers,
and power users with system operations. 

SMAH helps simplify tasks by providing command generation, 
pipe processing and other tasks tailored to your background and operating system. 

### Purpose

SMAH is designed to:
1. **Generate Commands**: Avoid endless searches through documentation. Simply describe what you want to do, and SMAH generates the necessary command and explains its potential impact. SMAH prints suggested commands for you to review and run yourself — it never executes them. You just want to use ffmpeg(tm) to convert a mp4 into a gif. 
2. **Assisted Pipe Generation**: Take advantage of SMAH’s ability to assist in generating complex pipes and chaining commands in Unix environments.
3. **AI-Assisted Monitoring (pending)**: Set up system monitoring tasks quickly with AI guidance, allowing you to keep track of critical metrics and thresholds without manual configuration.

### Key Features

- **Interactive Command Generation**: SMAH can guide you through setting up environments, managing packages, configuring system services, and more, by generating commands and explaining their implications; you review and run them yourself. And ask questions as it (pending) process your query.
- **User Preferences and Personalization**: SMAH remembers your preferences for certain tools (e.g., package managers, environment management tools) and uses those to generate optimal commands, saving time and reducing repetitive configuration.
- **Print-Only Commands (Safety)**: For safety, SMAH never runs commands. Each suggested command is printed in a clearly delimited block with an explanation of what it does and its expected impact, ready for you to copy and run yourself. If pyperclip is installed the command is also copied to your clipboard.
- **Model Planning/Prompt Planning**: A fast model is used to look at a sample of pipe data or your inquiry in order to decide
the best output format, runner model, additional instructions nad other details.


## Installation


### Pip Install
```
pip install smah
## call smah to begin setup. 
smah --configure
```

or 

### Local Build
@see [Contributing - Local Build Setup](CONTRIBUTING.md#local-development-setup)

-  git checkout code
- install poetry and python env. 
- setup python env (conda, venv, etc.)
- navigate to smah folder and run `poetry install`
- set SMAH_OPENAI_API_KEY env variable or set key in your smah config file `~/.smah/config.yaml`
- run smah --configure

## Examples

###  Pipe Input Processing

#### Log Audit to Pipe Output Format
Analyze Access Log Entries

```sh
tail -n 500 /var/log/auth.log | smah -q "Analyze these access log entries and output a markdown summary with tables of suspicious ips and events."  
```
![image](https://github.com/user-attachments/assets/445c0004-84db-46a9-b654-bb3c3692e73b)


#### System Control Summary
```
systemctl status | smah -i ~/SCAN-SYSTEMCTL.md
```
![image](https://github.com/user-attachments/assets/0304e06c-bb6f-4a2c-894d-f672225b1b83)



#### System Task
```sh
smah -q "How do I setup a web proxy on this server that I can connect to via my remote desktop to access the web?"
 ```
![image](https://github.com/user-attachments/assets/b712fbd7-7e2b-4e1f-9320-71c48fe90e58)

#### Resume last chat 

```sh
smah --continue
```

#### Resume recent or (pending) search old conversations to pick up from.

```sh
smah --history
```
![image](https://github.com/user-attachments/assets/6da1841e-5014-468b-b933-6b215853c80e)



#### Resume by id.

```sh
smah --session ID
```

#### Saved Prompt
```
systemctl status | smah -i ~/.scan-status.md 
```

#### Help
```
usage: smah [-h] [-q QUERY] [-i INSTRUCTIONS] [--continue | --no-continue]
            [--session SESSION] [--history | --no-history]
            [--interactive | --no-interactive] [--model MODEL]
            [--model-query MODEL_QUERY] [--model-pipe MODEL_PIPE]
            [--openai-api-key OPENAI_API_KEY] [--rich | --no-rich] [-c CONFIG]
            [--database DATABASE] [--configure | --no-configure] [-v]

SMAH: a system-aware command line assistant. Ask a question or pipe input through an LLM that knows your OS and preferences.
SMAH prints suggested commands with explanations; it never executes them.

options:
  -h, --help            show this help message and exit

input:
  -q QUERY, --query QUERY
                        The question or task to process (required unless
                        resuming a session)
  -i INSTRUCTIONS, --instructions INSTRUCTIONS
                        Path to a file whose contents are used as the query

sessions:
  --continue, --no-continue
                        Resume the most recent conversation
  --session SESSION     Resume a specific conversation by id
  --history, --no-history
                        Pick one of the 10 most recent conversations to resume
  --interactive, --no-interactive
                        Interactive chat mode (NOT IMPLEMENTED: exits with an
                        error)

models:
  --model MODEL         Override the model used to answer (e.g.
                        openai.gpt-4o-mini)
  --model-query MODEL_QUERY
                        Model used for the query planning stage
  --model-pipe MODEL_PIPE
                        Model used for the pipe planning stage
  --openai-api-key OPENAI_API_KEY
                        OpenAI API key (prefer the SMAH_OPENAI_API_KEY /
                        OPENAI_API_KEY env vars or the config file)

output:
  --rich, --no-rich     Render responses as rich Markdown instead of plain
                        text

configuration:
  -c CONFIG, --config CONFIG
                        Path to an alternative config file (default:
                        ~/.smah/config.yaml)
  --database DATABASE   Path to the sqlite history database (default:
                        ~/.smah/smah.db)
  --configure, --no-configure
                        Run the interactive setup wizard

diagnostics:
  -v, --verbose         Increase verbosity (repeatable: -v, -vv, -vvv). -v
                        shows plans and payloads, -vvv also dumps settings

examples:
  # summarize the tail of an auth log (pipe processing)
  tail -n 500 /var/log/auth.log | smah -q "Analyze these access log entries and output a markdown summary with tables of suspicious ips and events."

  # process piped output using a saved instruction file
  systemctl status | smah -i ~/SCAN-SYSTEMCTL.md

  # ask a plain question
  smah -q "How do I setup a web proxy on this server that I can connect to via my remote desktop to access the web?"

  # resume your most recent conversation
  smah --continue

input piping:
  When stdin is not a terminal its full contents are captured and sent along
  with your query (a query passed with -q or -i is still required).
```



## Setup

```
pip install smah
```

### Getting Started
When you run `smah` for the first time, it will guide you through a setup process to configure the necessary settings. Here is a step-by-step walkthrough of what the first run setup flow looks like:

#### 1. First Run
```sh
smah
```


## First Run Setup Flow

On the first run you will be asked about your background, experience,
and system details will be extracted or manually provided to reference 
future requests. 

system details will be extracted or manually provided, 
and the chance to set a optional instruction statement to include 
in every request. 

After completing these steps, your `smah` tool will be configured and ready to use.

#### 3. Fine Tuning
You can tweak model setting rules/weighthing currently by 
manually editing the generated `~/.smah/config.yaml` file. 
You can try differnt config options out with the `--config` arg


```yaml 
vsn: 0.0.1
user:
  vsn: 0.0.1
  name: Your Name
  system_admin_experience: Intermediate
  role: Administrator
  about: |
    I prefer detailed responses. I am well versed in biology, mathematics, and software testing and engineering, as well as relational database design and tuning. I am an INTP personality type.
system:
  vsn: 0.0.1
  operating_system:
    type: Linux
    name: posix
    version: '#1 SMP Thu Oct 5 21:02:42 UTC 2023'
    release: 5.15.133.1-microsoft-standard-WSL2
    info:
      vsn: 0.0.1
      kind: Linux
      source: os-release
      details:
        bug-report-url: https://bugs.launchpad.net/ubuntu/
        home-url: https://www.ubuntu.com/
        id: ubuntu
        id-like: debian
        name: Ubuntu
        pretty-name: Ubuntu 22.04.4 LTS
        privacy-policy-url: https://www.ubuntu.com/legal/terms-and-policies/privacy-policy
        support-url: https://help.ubuntu.com/
        ubuntu-codename: jammy
        version: 22.04.4 LTS (Jammy Jellyfish)
        version-codename: jammy
        version-id: '22.04'
    vsn: 0.0.1
inference:
  vsn: 0.0.1
  instructions: null # Optional additional instructions to include in each request.
  model_picker:
    default:
    - anthropic.claude-haiku-5-5
    - openai.gpt-4o-mini
    - openai.gpt-4o
  providers:
    openai:
      name: OpenAI
      description: OpenAI Models
      enabled: true
      vsn: 0.0.1
      settings: {}
      models:
      - name: gpt-4-turbo
        model: gpt-4-turbo
        description: GPT-4 Turbo
        enabled: false
        training_cutoff: 2023-12-01 00:00:00
        license: OpenAI License
        model_type: LLM
        context:
          window: 128000
          out: 4096
        strengths:
        - Highly accurate
        - Fast
        weaknesses:
        - Expensive
        - Multimodal capabilities limited
        modalities:
          text:
            in: true
            out: true
          image:
            in: true
            out: false
        settings: {}
        attributes:
          speed: 5
          reasoning: 5
          planning: 5
          creativity: 5
          conciseness: 5
          coding: 5
        cost:
          million_tokens_in: 10.0
          million_tokens_out: 30.0
        use_cases:
        - name: Code Generation
          score: 0.5
        - name: Text Generation
          score: 0.6
        - name: Translation
          score: 0.5
        - name: Planning
          score: 0.5
        - name: Reasoning
          score: 0.5
        - name: Creativity
          score: 0.5
        - name: Data Analysis
          score: 0.5
        vsn: 0.0.1
## ...        
      - name: o1-mini
        model: o1-mini
        description: o1 (mini)
        enabled: false
        training_cutoff: 2023-12-01 00:00:00
        license: OpenAI License
        model_type: LLM
        context:
          window: 128000
          out: 32768
        strengths:
        - Highly accurate
        - Advanced Reasoning
        weaknesses:
        - Expensive
        - Multimodal capabilities limited
        modalities:
          text:
            in: true
            out: true
        settings:
          max_completion_tokens: true
        attributes:
          speed: 5
          reasoning: 7
          planning: 7
          creativity: 4
          conciseness: 7
          coding: 7
        cost:
          million_tokens_in: 3.0
          million_tokens_out: 12.0
        use_cases:
        - name: Code Generation
          score: 0.7
        - name: Text Generation
          score: 0.7
        - name: Translation
          score: 0.7
        - name: Planning
          score: 0.7
        - name: Reasoning
          score: 0.7
        - name: Creativity
          score: 0.4
        - name: Data Analysis
          score: 0.7
        vsn: 0.0.1

```

## Security
Do not pass sensitive data/details to Smah if your LLM will persist/track it, use a (pending) local model instead.
Double check exec commands they may have errors and require edits prior to execution. 

### Future Enhancements

SMAH is in its early alpha stage, and we are actively working on the following enhancements:

1. **Expanded Command Generation Use Cases**: Increase the range of commands that SMAH can generate, including more complex system operations and integrations with additional tools.
2. **Enhanced AI-Assisted Monitoring**: Improve the AI's ability to set up and manage system monitoring tasks, including more advanced metrics and alerting capabilities.
3. **Interactive Rich Terminal Interface**: Develop a terminal-based user interface for managing and interacting with the system, providing a richer user experience.
4. **Multi-User Support**: Implement support for multiple user accounts with shared global configuration options, allowing for seamless management across different users.
5. **Advanced Security Features**: Strengthen security measures, especially when running commands with elevated privileges, and enhance guardrail enforcement.
6. **Integration with External Tools**: Enable integration with other popular tools and services to streamline workflows and improve productivity.
7. **Data Visualization**: Add capabilities for visualizing data trends and analysis results, making it easier to interpret and present findings.
8. **Automated Reporting**: Develop features for generating automated reports based on data analyses, saving time on repetitive tasks.
9. **Enhanced Logging and Monitoring**: Improve the logging and monitoring system to provide more detailed insights and better auditability.
10. **Secrets Management**: Implement secure storage and management of sensitive information, such as API keys and passwords.

### 1-Year Roadmap

#### Q1
- **Release Beta Version**: Transition from alpha to beta, incorporating initial user feedback and stabilizing core features.
- **Expand Command Generation**: Add support for more complex commands and integrations with additional tools.
- **Improve AI-Assisted Monitoring**: Enhance the AI's capabilities for setting up and managing system monitoring tasks.

#### Q2
- **Develop Interactive Terminal Interface**: Create a rich terminal-based user interface for better user interaction.
- **Implement Multi-User Support**: Enable support for multiple user accounts with shared global configuration options.
- **Strengthen Security Features**: Enhance security measures, especially for commands requiring elevated privileges.

#### Q3
- **Integrate with External Tools**: Enable seamless integration with other popular tools and services.
- **Add Data Visualization**: Develop features for visualizing data trends and analysis results.
- **Automate Reporting**: Implement automated reporting capabilities to save time on repetitive tasks.

#### Q4
- **Enhance Logging and Monitoring**: Improve the logging and monitoring system for better insights and auditability.
- **Implement Secrets Management**: Develop secure storage and management of sensitive information.
- **Prepare for General Availability**: Finalize features and prepare for a general availability release, ensuring stability and robustness.

## Contributing

We welcome contributions to help improve SMAH. Here are some guidelines to get you started:

### How to Contribute

#### Feature Requests, Feedback and Bug Reports

- Use the GitHub issues tracker to report bugs or suggest features.
- Provide as much detail as possible, including steps to reproduce the issue.
_ When use user-story style at the openning of feature/change requests.

#### Bug Fixes and Code Changes
 
1. Fork the Repository: Create a personal fork of the repository on GitHub.
2. Create a Feature: Create a new branch for your feature or bugfix.
3. Make Changes: Make your changes in the new branch.
4. Commit Changes: Use descriptive commit changes orgnanize work, 
   rebase/group related items together for complex multi part changes
   and split into sub branches/pull requests. 
5. Make A Pull Request - give details on what you've changed/fixed.

#### Code Style

- When Possible Follow the existing code style and conventions unless overhauling format/layout only (I suggest asking first.) 
- Write clear, concise, and descriptive commit messages.
- Use Type Hints and Doc Tags. 
- Ensure your code is well-documented. Consider md and mermaid diagrams for complax/major changes.

#### Testing

- Write pytests for any new features or bug fixes.
- Ensure all tests pass before submitting a pull request.


### Code of Conduct

Be Kind.

## License
[MIT](./LICENSE)
