%%% ====================================================================
%%%  @LaTeX-class-file{
%%%	filename	= "unswthesis.cls",
%%%	version		= "2.0.1",
%%%	date		= "30 April 2015",
%%%	writtenby	= "Geoff Oakley",
%%%	email		= "geoffo@unsw.edu.au",
%%%	keywords	= "LaTeX, thesis, cse, unsw, unswthesis",
%%%	docstring	= "This file provides a thesis class for the
%%%			   Faculty of Engineering,
%%%			   University of New South Wales, Australia."
%%% }
%%% ====================================================================

% This file provides a thesis class for the Faculty of
% Engineering, University of New South Wales.  It is meant to be
% used with LaTeX-2e, and exists because of the archaic standards
% required by universities all over the world with regards to thesis
% styles.
%
% As with all class files, this class is meant to be used as the exclusive
% LaTeX-2e class for the entire document.  It may be introduced in the
% document via:
%
%       \documentclass{unswthesis}
%
% As always, options may be placed before the {unswthesis}.  In this case,
% the following options are supported:
%
%       singlespacing   - Single-space the document (ie, do not apply the
%                         thesis requirements).
%       doublespacing   - Double-space the document (actually, space it
%                         1.5 times the baseline).
%       draft           - Prints the word "DRAFT" across the front page,
%                         as well as on each page (in the footer).
%       final           - Opposite of draft: do not print "DRAFT". Default.
%
%
%	undergrad       - sets margins compatible with minimum requirements,
%                       - doesn't include disclaimer.
%                       - Default mode.
%
%	hdr		- sets margins compatible with minimum requirements,
%			- sets double spacing by default,
%			- adds UNSW disclaimer to the inside cover,
%
%


%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
\NeedsTeXFormat{LaTeX2e}
\ProvidesClass{unswthesis}%
    [2015/04/30 v2.0.1 Faculty of Engineering, University of NSW Thesis Class]
\RequirePackage{fancyhdr}
\RequirePackage{graphics}
\RequirePackage{units}

%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
%   Changeable Parameters   %
%%%%%%%%%%%%%%%%%%%%%%%%%%%%%

% Change these for customisation.  To change them in a LaTeX document,
% simply use \thesisfaculty{xxx} and \thesisschool{xxx}.

\gdef\ThesisFacultyName{Faculty of Engineering}
\gdef\ThesisSchoolName{Name of School}

% Customise for foreign languages.

\def\supervisorname{Supervisor}
\def\abstractname{Abstract}
\def\declarationname{Originality Statement}

%%%%%%%%%%%%%%%%%%%%%
%   Class options   %
%%%%%%%%%%%%%%%%%%%%%

% New defns for ugrad and hdr - andrewo
% ugrad defaults to true, is set false for phd option
\newif\if@ugrad \@ugradtrue

\newif\if@dspacing{}
\newif\if@thdraft{}
\def\f@ntsize{11pt}   % default font size
\def\sidefl@g{}

% new option for ugrad/hdr selection
\DeclareOption{undergrad}{\@ugradtrue}
\DeclareOption{hdr}{\@ugradfalse}

\DeclareOption{11pt}{\def\f@ntsize{11pt}}
\DeclareOption{12pt}{\def\f@ntsize{12pt}}
\DeclareOption{singlespacing}{\@dspacingfalse}
\DeclareOption{doublespacing}{\@dspacingtrue}
\DeclareOption{oneside}{\def\sidefl@g{oneside}}
\DeclareOption{twoside}{\def\sidefl@g{twoside}}
\DeclareOption{draft}{\@thdrafttrue}
\DeclareOption{final}{\@thdraftfalse}
%\DeclareOption*{\PassOptionsToClass{\CurrentOption}{book}}
%default values go here - overridden below
\ExecuteOptions{doublespacing,final}
\ProcessOptions

\ifx\sidefl@g\empty
  % neither oneside nor twoside were selected,
  % set default for undergrad and hdr thesis
  \if@ugrad \def\sidefl@g{twoside}
  \else \def\sidefl@g{oneside}
  \fi
\fi

\LoadClass[a4paper,\sidefl@g,\f@ntsize,openany]{book}

%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
%   Draft / final options   %
%%%%%%%%%%%%%%%%%%%%%%%%%%%%%

\if@thdraft%
  \RequirePackage[firstpage]{draftwatermark}
  \def\draftfont{\fontfamily{phv}\fontsize{48pt}{60pt}\selectfont}
  \SetWatermarkFontSize{12pt} % we don't use this, but it sometimes
                              % triggers a font error without it 
  \SetWatermarkText{\draftfont DRAFT --- \today\hskip50mm \vrule height 60mm width 0mm}
\fi

%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
%   Double and single spacing   %
%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%

\newcommand{\ssp}{%
    \def\baselinestretch{1}%
    \normalsize\selectfont}
\newcommand{\dsp}{%
    \def\baselinestretch{1.5}%{1.21}%
    \normalsize\selectfont}

% Define conditional:

\if@dspacing%
    \newcommand\s@dsp{\dsp}%
\else%
    \newcommand\s@dsp{\ssp}%
\fi

% Conditional Double-spacing environment
% Not sure if these are particularly useful,
%  and blithe use of spingle spacing should be discouraged

\def\spacing{%
    \begingroup%
    \s@dsp}%
\def\endspacing{%
    \par \vskip \parskip \vskip \baselineskip%
    \endgroup%
    \vskip -\parskip%
    \vskip -\baselineskip}

% Always single-space

\def\sspacing{%
    \begingroup%
    \ssp}%
\def\endsspacing{%
    \par \vskip \parskip \vskip \baselineskip%
    \endgroup%
    \vskip -\parskip%
    \vskip -\baselineskip}

% Execute the following ONLY if double-spacing was selected.

\if@dspacing
    % double spaceing for mainmatter
    \let\dsp@mainmatter=\mainmatter
    \renewcommand{\mainmatter}{\dsp@mainmatter\clearemptydoublepage\s@dsp}

    % single spacing for backmatter
    \let\dsp@backmatter=\backmatter
    \renewcommand{\backmatter}{\dsp@backmatter\ssp}

    % single spacing for footnotes
    \let\dsp@makefntext=\@makefntext
    \long\def\@makefntext#1{\ssp\dsp@makefntext{#1}}

    % single spacing for figure and table text
    % the following are from "book.cls":
    \renewenvironment{figure}[1][tbp]{\@float{figure}[#1]\ssp}{\end@float\s@dsp}
    \renewenvironment{figure*}[1][tbp]{\@dblfloat{figure}[#1]\ssp}{\end@dblfloat\s@dsp}
    \renewenvironment{table}[1][tbp]{\@float{table}[#1]\ssp}{\end@float\s@dsp}
    \renewenvironment{table*}[1][tbp]{\@dblfloat{table}[#1]\ssp}{\end@dblfloat\s@dsp}
\fi


%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
%   Miscellaneous Commands   %
%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%

\newcommand{\thesisfacultyl}[1]{\gdef\ThesisFacultyName{#1}}
\newcommand{\thesisschool}[1]{\gdef\ThesisSchoolName{#1}}
\newcommand{\thesistitle}[1]{\gdef\ThesisTitle{#1}}
\newcommand{\thesisauthor}[1]{\gdef\ThesisAuthor{#1}}
\newcommand{\thesisZid}[1]{\gdef\ThesisAuthorZid{#1}}
\newcommand{\thesisdegree}[1]{\gdef\ThesisDegreeName{#1}}
\newcommand{\thesisdate}[1]{\gdef\ThesisDate{#1}}
\newcommand{\thesissupervisor}[1]{\gdef\ThesisSupervisorName{#1}}

\newcommand{\ThesisTitle}{\@latex@error{No \noexpand\thesistitle given}\@ehc}
\newcommand{\ThesisAuthor}{\@latex@error{No \noexpand\thesisauthor given}\@ehc}
\newcommand{\ThesisAuthorZid}{\@latex@error{No \noexpand\thesisZid given}\@ehc}
\newcommand{\ThesisDegreeName}{\@latex@error{No \noexpand\thesisdegree given}\@ehc}
\newcommand{\ThesisDate}{\@latex@error{No \noexpand\thesisdate given}\@ehc}
\newcommand{\ThesisSupervisorName}{\@latex@error{No \noexpand\thesissupervisor given}\@ehc}

\newcommand{\ThesisInsideMark}{}
\newcommand{\ThesisOutsideMark}{}

\renewcommand{\title}[1]{\@latex@warning{\noexpand\title is not used with this class}}
\renewcommand{\author}[1]{\@latex@warning{\noexpand\author is not used with this class}}
\renewcommand{\date}[1]{\@latex@warning{\noexpand\date is not used with this class}}


%%%%%%%%%%%%%%%%%%%%%%%%%%%
%   Thesis front matter   %
%%%%%%%%%%%%%%%%%%%%%%%%%%%

\newcommand{\clearemptydoublepage}{\newpage{\pagestyle{fancy}\cleardoublepage}}

% Declaration for HDR (typically PhD) class.
\newcommand{\ThesisFamilyName}[1]{\gdef\ThesisFamilyNameString{#1}}
\newcommand{\ThesisGivenName}[1]{\gdef\ThesisGivenNameString{#1}}
\newcommand{\ThesisOtherNames}[1]{\gdef\ThesisOtherNamesString{#1}}
\newcommand{\ThesisAbbreviatedDegree}[1]{\gdef\ThesisAbbreviatedDegreeString{#1}}

\ThesisFamilyName{*not*set*}
\ThesisGivenName{*not*set*}
\ThesisOtherNames{*not*set*} %if any
\ThesisAbbreviatedDegree{*not*set*}

\usepackage{collect}
\definecollection{@bstract}
\newenvironment{ThesisAbstract}%
   {\@nameuse{collect}{@bstract}{}{}}
   {\@nameuse{endcollect}}

\usepackage[]{geometry}%
\newcommand{\ThesisDissertationSheet}%
{%
\InputIfFileExists{dissertation-sheet}{}{\@latex@error{File
    dissertation-sheet.tex not available (required for HDR theses)}\@ehc}
\newgeometry{body={180mm,280mm},top=10mm,left=20mm}%
{\sffamily\footnotesize
\titlepage
%
\fbox{%
  \begin{minipage}{175mm}%
    \setlength{\parskip}{\medskipamount}
    \begin{center}
       {\bfseries THE UNIVERSITY OF NEW SOUTH WALES\\
        Thesis/Dissertation Sheet}
    \end{center}
    
    Surname or Family name: {\small\bfseries \ThesisFamilyNameString}
    
    First name: {\small\bfseries \ThesisGivenNameString}\hspace{10mm}
    Other name/s: {\small\bfseries \ThesisOtherNamesString}
    
    Abreviation for degree as given in the University calendar:
    {\small\bfseries \ThesisAbbreviatedDegreeString}
    
    School: {\small\bfseries \ThesisSchoolName}\hfill
    Faculty: {\small\bfseries \ThesisFacultyName}
    
    Title: {\small\ThesisTitle}
  \end{minipage}
}

\fbox{%
  \begin{minipage}[t]{175mm}%
    \setlength{\parskip}{\medskipamount}
    \begin{center}
       {\bfseries Abstract 350 words maximum}
    \end{center}
    \vspace{-3ex}

    {\normalsize\includecollection{@bstract}\par}

  \end{minipage}
  \vrule width 0mm depth 140mm\relax % force box to be at least 140mm high
}

\fbox{%
  \begin{minipage}{175mm}%
    \setlength{\parskip}{\medskipamount}
    {\bfseries Declaration relating to disposition of project thesis/dissertation}

I hereby grant to the University of New South Wales or its agents the
right to archive and to make available my thesis or dissertation in
whole or in part in the University libraries in all forms of media,
now or here after known, subject to the provisions of the Copyright
Act 1968. I retain all property rights, such as patent rights. I also
retain the right to use in future works (such as articles or books)
all or part of this thesis or dissertation.

I also authorise University Microfilms to use the 350 word abstract of
my thesis in Dissertation Abstracts International (this is applicable
to doctoral theses only).
\vspace*{10mm}

\hfil
Signature
\hfil\hfil\hfil
Witness
\hfil\hfil\hfil
Date
\hfil

The University recognises that there may be exceptional circumstances
requiring restrictions on copying or conditions on use. Requests for
restriction for a period of up to 2 years must be made in
writing. Requests for a longer period of restriction may be considered
in exceptional circumstances and require the approval of the Dean of
Graduate Research.

  \end{minipage}
}

\fbox{%
  \begin{minipage}{175mm}%
    \setlength{\parskip}{\medskipamount}
    {\bfseries FOR OFFICE USE ONLY}
    \hfill Date of completion of requirements for Award
    \hfill
\vspace*{10mm}

  \end{minipage}
}
\clearemptydoublepage
}\endtitlepage
\restoregeometry
}

\newcommand{\smallhe@d}{\Large\bfseries}

\newcommand{\ThesisOriginalityDeclaration}%
{\titlepage
{\smallhe@d \declarationname\par}
\vskip 3ex

{\leavevmode \llap{`}I hereby declare that this submission is my
  own work and to the best of my knowledge it contains no materials
  previously published or written by another person, or substantial
  proportions of material which have been accepted for the award of
  any other degree or diploma at UNSW or any other educational
  institution, except where due acknowledgement is made in the
  thesis. Any contribution made to the research by others, with whom I
  have worked at UNSW or elsewhere, is explicitly acknowledged in the
  thesis. I also declare that the intellectual content of this thesis
  is the product of my own work, except to the extent that assistance
  from others in the project's design and conception or in style,
  presentation and linguistic expression is acknowledged.'  }
\vskip 8em%
\begin{flushleft}
\ThesisAuthor\\
\today
\end{flushleft}
\vfil
\clearemptydoublepage
\endtitlepage
}

% Digital Copyright and Authenticity Statements for HDR (typically PhD) class.
\newcommand{\CopyrightAndAuthenticity}%
{\titlepage
{\smallhe@d Copyright Statement\par}
\vskip 3ex

\leavevmode \llap{`}I hereby grant the University of New South Wales
  or its agents the right to archive and to make available my thesis
  or dissertation in whole or part in the University libraries in all
  forms of media, now or here after known, subject to the provisions
  of the Copyright Act 1968. I retain all proprietary rights, such as
  patent rights. I also retain the right to use in future works (such
  as articles or books) all or part of this thesis or dissertation.  

  I also authorise University Microfilms to use the 350 word abstract of
  my thesis in Dissertation Abstract International (this is applicable
  to doctoral theses only).  

  I have either used no substantial portions of copyright material in
  my thesis or I have obtained permission to use copyright material;
  where permission has not been granted I have applied/will apply for
  a partial restriction of the digital copy of my thesis or
  dissertation.'
\vskip 10ex%
\begin{flushleft}
\ThesisAuthor\\
\today
\end{flushleft}
\vfill

{\smallhe@d Authenticity Statement\par}
\vskip 3ex

{\leavevmode \llap{`}I certify that the Library deposit digital copy
  is a direct equivalent of the final officially approved version of
  my thesis. No emendation of content has occurred and if there are
  any minor variations in formatting, they are the result of the
  conversion to digital format.'  }
\vskip 10ex%
\begin{flushleft}
\ThesisAuthor\\
\today
\end{flushleft}
\clearemptydoublepage
\endtitlepage
}

\def\coverpagetextfont{\fontfamily{phv}\fontsize{11pt}{18pt}\selectfont}
\def\coverpagefont{\fontfamily{phv}\fontsize{14pt}{20pt}\selectfont}
\def\boldcoverpagefont{\fontfamily{phv}\fontseries{b}\fontsize{14pt}{20pt}\selectfont}
\def\largecoverpagefont{\fontfamily{phv}\fontsize{22pt}{32pt}\selectfont}
\def\largeboldcoverpagefont{\fontfamily{phv}\fontseries{b}\fontsize{22pt}{36pt}\selectfont}
\def\largeslantedcoverpagefont{\fontfamily{phv}\fontshape{sl}\fontsize{22pt}{32pt}\selectfont}

\if@ugrad
\renewcommand{\maketitle}%
    {\begin{titlepage}%
        \vspace*{-20mm}%
        \enlargethispage{\footskip}% no footer on cover
	\begin{center}%
            \coverpagefont%
            \includegraphics{crest}%
	    \vskip 0pt plus 8fill
	    {\boldcoverpagefont \ThesisSchoolName\par}%
	    {\boldcoverpagefont \ThesisFacultyName\par}%
	    {\boldcoverpagefont The University of New South Wales\par}%
	    \vskip 0pt plus 8fill%
	    {\largeboldcoverpagefont \ThesisTitle \par}%
	    \vskip 0pt plus 5fill%
            by
	    \vskip 0pt plus 4fill%
	    {\largecoverpagefont \ThesisAuthor \par}%
            \vskip 0pt plus 12fill
	    Thesis submitted as a requirement for the degree of\\[1ex]
            \ThesisDegreeName \par%
	    \vskip 0pt plus 10fill
            Submitted: \ThesisDate \vrule width 0pt depth 10pt\\
            \begin{tabular}{@{}rl}              
              Supervisor: & \ThesisSupervisorName
            \end{tabular}
            \hfill
            \begin{tabular}{rl@{}}
              Student ID: & \ThesisAuthorZid \vrule width 0pt depth 10pt
            \end{tabular}\par
	\end{center}\par
    \end{titlepage}}%
\else
\renewcommand{\maketitle}%
    {\begin{titlepage}%
        \vspace*{-20mm}%
        \enlargethispage{\footskip}% no footer on cover
	\begin{center}%
          \coverpagefont
          {\largeboldcoverpagefont \ThesisTitle \par}%
          \vskip 0pt plus 8fill
          {\largecoverpagefont \ThesisAuthor \par}%
          \vskip 0pt plus 8fill
          { A thesis in fulfillment of the requirements for the degree of \\[2ex]
              \ThesisDegreeName \par}%
          \vskip 0pt plus 8fill
          \includegraphics{crest}
	  \vskip 0pt plus 4fill
          \ThesisSchoolName\par
          \ThesisFacultyName\par
          The University of New South Wales\par
	  \vskip 0pt plus 4fill
          \ThesisDate \par%
	\end{center}\par
    \end{titlepage}
    \clearemptydoublepage
    \ThesisDissertationSheet
    \ThesisOriginalityDeclaration
    \CopyrightAndAuthenticity
    \setcounter{footnote}{0}
    \let\maketitle\relax}
\fi

\renewenvironment{theindex}
{ \columnseprule \z@ \columnsep 35\p@
  \twocolumn[\@makeschapterhead{\indexname}]%
  \addcontentsline{toc}{chapter}{Index}
  \@mkboth{\uppercase{\indexname}}{\uppercase{\indexname}}%
  \thispagestyle{plain}\parindent\z@
  \parskip\z@ \@plus .3\p@\relax
  \let\item\@idxitem}
  {\onecolumn}


%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
%   General Page Parameters   %
%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%

% The university minimum margin requirements for theses are as follows

\if@ugrad
	%undergrad dimensions
	\setlength{\oddsidemargin}{40mm}
	\setlength{\evensidemargin}{25mm}
	\setlength{\topmargin}{25mm}
	\setlength{\headsep}{20pt}
	\setlength{\headheight}{25pt}
\else
	%hdr dimensions
	\setlength{\oddsidemargin}{40mm}
	\setlength{\evensidemargin}{20mm}
	\setlength{\topmargin}{30mm}
	\setlength{\headsep}{20pt}
	\setlength{\headheight}{25pt}
\fi
	
\newlength\botmargin
\setlength{\botmargin}{20mm}

\setlength{\marginparwidth}{25mm}
\setlength{\marginparsep}{7\p@}

\setlength{\footskip}{15mm}
\setlength{\footnotesep}{14\p@}

\setlength{\parskip}{1em plus 0.5ex}
\setlength{\parindent}{0pt}

\setlength{\textheight}{\paperheight}
  \addtolength{\textheight}{-\topmargin}
  \addtolength{\textheight}{-\headheight}
  \addtolength{\textheight}{-\headsep}
  \addtolength{\textheight}{-\footskip}
  \addtolength{\textheight}{-\botmargin}
\setlength{\textwidth}{\paperwidth}
  \addtolength{\textwidth}{-\oddsidemargin}
  \addtolength{\textwidth}{-\evensidemargin}

% now reset margins relative to Kunthian 1in,1in
\addtolength{\topmargin}{-1in}
\addtolength{\oddsidemargin}{-1in}
\addtolength{\evensidemargin}{-1in}

% Page headings and footers.
\pagestyle{fancyplain}
\fancyhead{}
\fancyfoot{}
\renewcommand{\headrulewidth}{0pt}
\cfoot{\fancyplain{\textrm{\thepage}}{\textrm{\thepage}}}

\if@thdraft
    \lfoot[\fancyplain{\textbf{Draft:}~\today}{\textbf{Draft:}~\today}]{}
    \rfoot[]{\fancyplain{\textbf{Draft:}~\today}{\textbf{Draft:}~\today}}
\fi



%%% Changes to backmatter such that
%%%  \chapter generates alpha-numbered chapters starting with A

\newif\ifb@ckmatter \b@ckmatterfalse

\let\alpha@backmatter=\backmatter
\renewcommand\backmatter{%
  \setcounter{chapter}{0}%
  \renewcommand*{\thechapter}{\Alph{chapter}}%
  \b@ckmattertrue%
  \alpha@backmatter}

\let\back@chapter=\chapter
\renewcommand\chapter{%
  \secdef{\ifb@ckmatter\refstepcounter{chapter}\fi\back@chapter}{\back@chapter*}}

%%% and add a TOC line for the bibliography

\let\toc@bibliography=\bibliography
\renewcommand\bibliography[1]{%
  \addcontentsline{toc}{chapter}{Bibliography}%
  \toc@bibliography{#1}}
